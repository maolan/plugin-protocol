use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Magic number: "MAOL" in big-endian ASCII.
pub const MAGIC: u32 = 0x4D41_4F4C;

/// Current protocol version.
/// Version 2: parent_window changed from AtomicU32 to AtomicU64 to support 64-bit HWNDs on Windows.
/// Version 3: Added MIDI output ring for plugin-generated MIDI events.
pub const VERSION: u32 = 3;

/// Maximum number of audio channels (main + sidechain combined).
pub const MAX_CHANNELS: usize = 32;

/// Number of audio buses (main + sidechain).
pub const NUM_BUSES: usize = 2;

/// Maximum audio block size in samples.
pub const MAX_BLOCK_SIZE: usize = 4096;

/// Capacity of each ring buffer in slots (power of two).
pub const RING_CAPACITY: usize = 4096;

// --- Section sizes ---
pub const HEADER_SIZE: usize = 256;
pub const CONTROL_SIZE: usize = 256;
pub const AUDIO_BUFFER_SIZE: usize = MAX_CHANNELS * NUM_BUSES * MAX_BLOCK_SIZE * 4; // f32
pub const PARAM_RING_SIZE: usize = RING_CAPACITY * std::mem::size_of::<ParameterEvent>();
pub const MIDI_RING_SIZE: usize = RING_CAPACITY * std::mem::size_of::<MidiEvent>();
pub const TRANSPORT_SIZE: usize = 256;
pub const SCRATCH_SIZE: usize = 65536;

// --- Offsets into the shared-memory segment ---
/// Control area starts right after the header.
pub const CONTROL_OFFSET: usize = HEADER_SIZE;
/// Audio buffers start after the control area.
pub const AUDIO_OFFSET: usize = HEADER_SIZE + CONTROL_SIZE;
/// Parameter ring buffer.
pub const PARAM_RING_OFFSET: usize = AUDIO_OFFSET + AUDIO_BUFFER_SIZE;
/// MIDI ring buffer.
pub const MIDI_RING_OFFSET: usize = PARAM_RING_OFFSET + PARAM_RING_SIZE;
pub const ECHO_RING_OFFSET: usize = MIDI_RING_OFFSET + MIDI_RING_SIZE;
pub const ECHO_RING_SIZE: usize = RING_CAPACITY * std::mem::size_of::<ParameterEvent>();
pub const MIDI_OUT_RING_OFFSET: usize = ECHO_RING_OFFSET + ECHO_RING_SIZE;
pub const MIDI_OUT_RING_SIZE: usize = RING_CAPACITY * std::mem::size_of::<MidiEvent>();
/// Transport state block (256-byte aligned from here).
pub const TRANSPORT_OFFSET: usize = {
    let end = MIDI_OUT_RING_OFFSET + MIDI_OUT_RING_SIZE;
    // Align up to 256 bytes
    (end + 255) & !255
};
/// State blob scratch area.
pub const SCRATCH_OFFSET: usize = TRANSPORT_OFFSET + TRANSPORT_SIZE;

/// Total bytes actively used by the protocol layout.
pub const LAYOUT_SIZE: usize = SCRATCH_OFFSET + SCRATCH_SIZE;

/// Total shared-memory allocation size (4 MiB, page-aligned).
pub const SHM_SIZE: usize = 4 * 1024 * 1024;

// --- Control-area indices (all 4-byte atomics inside CONTROL_OFFSET..CONTROL_OFFSET+256) ---
pub const PARAM_WRITE_IDX_OFFSET: usize = CONTROL_OFFSET;
pub const PARAM_READ_IDX_OFFSET: usize = CONTROL_OFFSET + 4;
pub const MIDI_WRITE_IDX_OFFSET: usize = CONTROL_OFFSET + 8;
pub const MIDI_READ_IDX_OFFSET: usize = CONTROL_OFFSET + 12;
pub const ECHO_WRITE_IDX_OFFSET: usize = CONTROL_OFFSET + 16;
pub const ECHO_READ_IDX_OFFSET: usize = CONTROL_OFFSET + 20;
pub const MIDI_OUT_WRITE_IDX_OFFSET: usize = CONTROL_OFFSET + 24;
pub const MIDI_OUT_READ_IDX_OFFSET: usize = CONTROL_OFFSET + 28;

// --- Structs ---

pub const PARAM_EVENT_VALUE: u32 = 0;
pub const PARAM_EVENT_MOD: u32 = 1;
pub const PARAM_EVENT_GESTURE_BEGIN: u32 = 2;
pub const PARAM_EVENT_GESTURE_END: u32 = 3;

/// Fixed-size parameter change event (16 bytes, 16-byte aligned).
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default)]
pub struct ParameterEvent {
    pub param_index: u32,
    pub value: f32,
    pub sample_offset: u32,
    pub event_kind: u32,
}

/// Fixed-size MIDI event (16 bytes, 16-byte aligned).
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default)]
pub struct MidiEvent {
    pub sample_offset: u32,
    pub data: [u8; 3],
    pub channel: u8,
    pub flags: u16,
    pub _pad: u16,
}

/// Transport state block (256 bytes).
#[repr(C, align(256))]
#[derive(Clone, Copy, Debug)]
pub struct TransportState {
    pub playhead_sample: u64,
    pub tempo: f64,
    pub numerator: u32,
    pub denominator: u32,
    pub flags: u32,
    pub sample_rate_hz: f64,
    _pad: [u8; 256 - 40],
}

impl Default for TransportState {
    fn default() -> Self {
        Self {
            playhead_sample: 0,
            tempo: 120.0,
            numerator: 4,
            denominator: 4,
            flags: 0,
            sample_rate_hz: 0.0,
            _pad: [0; 256 - 40],
        }
    }
}

/// Shared-memory header (256 bytes).
#[repr(C, align(256))]
pub struct ShmHeader {
    pub magic: u32,
    pub version: u32,
    pub flags: u32,
    pub ready: AtomicU32,
    pub heartbeat: AtomicU32,
    pub error_code: u32,
    pub shutdown_request: AtomicU32,
    pub tasks_issued: AtomicU32,
    pub tasks_completed: AtomicU32,
    pub block_size: AtomicU32,
    pub num_input_channels: AtomicU32,
    pub num_output_channels: AtomicU32,
    /// Request type: 0 = none, 1 = save_state, 2 = restore_state, 3 = gui_show, 4 = gui_hide
    pub request_type: AtomicU32,
    /// Request status: 0 = pending, 1 = success, 2 = error
    pub request_status: AtomicU32,
    /// Valid bytes in scratch area for state operations
    pub scratch_size: AtomicU32,
    /// Parent window ID for GUI embedding (X11 window ID on Unix, HWND on Windows)
    pub parent_window: AtomicU64,
    _pad: [u8; 256 - 72],
}

impl ShmHeader {
    /// Load parent_window as a `usize` (handles 32- and 64-bit platforms).
    pub fn parent_window_usize(&self) -> usize {
        self.parent_window.load(Ordering::Acquire) as usize
    }

    /// Store a `usize` parent_window (truncates on 32-bit, but HWNDs/XIDs are
    /// always within 64 bits).
    pub fn set_parent_window(&self, window: usize) {
        self.parent_window.store(window as u64, Ordering::Release);
    }
}

impl Default for ShmHeader {
    fn default() -> Self {
        Self {
            magic: MAGIC,
            version: VERSION,
            flags: 0,
            ready: AtomicU32::new(0),
            heartbeat: AtomicU32::new(0),
            error_code: 0,
            shutdown_request: AtomicU32::new(0),
            tasks_issued: AtomicU32::new(0),
            tasks_completed: AtomicU32::new(0),
            block_size: AtomicU32::new(0),
            num_input_channels: AtomicU32::new(0),
            num_output_channels: AtomicU32::new(0),
            request_type: AtomicU32::new(0),
            request_status: AtomicU32::new(0),
            scratch_size: AtomicU32::new(0),
            parent_window: AtomicU64::new(0),
            _pad: [0; 256 - 72],
        }
    }
}

// --- Layout helpers ---

/// Zero-initialize the entire shared-memory region and write the header.
///
/// # Safety
/// `ptr` must be a valid pointer to a memory region of `size` bytes.
pub unsafe fn init_shm_layout(ptr: *mut u8, size: usize) {
    unsafe {
        std::ptr::write_bytes(ptr, 0, size);
        let header = ptr as *mut ShmHeader;
        std::ptr::write(header, ShmHeader::default());
    }
}

/// Returns a reference to the header at the start of the mapping.
///
/// # Safety
/// `ptr` must point to a valid allocation containing at least `ShmHeader`'s size.
pub unsafe fn header_ref(ptr: *mut u8) -> &'static ShmHeader {
    unsafe { &*(ptr as *mut ShmHeader) }
}

/// Returns a mutable reference to the header.
///
/// # Safety
/// `ptr` must point to a valid allocation containing at least `ShmHeader`'s size.
pub unsafe fn header_mut(ptr: *mut u8) -> &'static mut ShmHeader {
    unsafe { &mut *(ptr as *mut ShmHeader) }
}

/// Returns a pointer to the audio buffer region.
///
/// # Safety
/// `ptr` must point to an allocation large enough to contain the audio buffer.
pub unsafe fn audio_ptr(ptr: *mut u8) -> *mut f32 {
    unsafe { ptr.add(AUDIO_OFFSET) as *mut f32 }
}

/// Returns a pointer to a specific channel/bus plane.
///
/// `channel` is 0-based up to `MAX_CHANNELS - 1`.
/// `bus` is 0 (main) or 1 (sidechain).
///
/// # Safety
/// `ptr` must point to a valid allocation large enough to contain the audio data.
pub unsafe fn audio_channel_ptr(ptr: *mut u8, channel: usize, bus: usize) -> *mut f32 {
    let plane_size = MAX_BLOCK_SIZE * std::mem::size_of::<f32>();
    let offset = AUDIO_OFFSET + (channel * NUM_BUSES + bus) * plane_size;
    unsafe { ptr.add(offset) as *mut f32 }
}

/// Returns a pointer to the parameter ring buffer slot array.
///
/// # Safety
/// `ptr` must point to a valid allocation large enough to contain the parameter ring.
pub unsafe fn param_ring_ptr(ptr: *mut u8) -> *mut ParameterEvent {
    unsafe { ptr.add(PARAM_RING_OFFSET) as *mut ParameterEvent }
}

/// Returns pointers to the parameter ring write/read atomics.
///
/// # Safety
/// `ptr` must point to a valid allocation containing the parameter ring atomics.
pub unsafe fn param_indices(ptr: *mut u8) -> (*mut AtomicU32, *mut AtomicU32) {
    unsafe {
        (
            ptr.add(PARAM_WRITE_IDX_OFFSET) as *mut AtomicU32,
            ptr.add(PARAM_READ_IDX_OFFSET) as *mut AtomicU32,
        )
    }
}

/// Returns a pointer to the MIDI ring buffer slot array.
///
/// # Safety
/// `ptr` must point to a valid allocation large enough to contain the MIDI ring.
pub unsafe fn midi_ring_ptr(ptr: *mut u8) -> *mut MidiEvent {
    unsafe { ptr.add(MIDI_RING_OFFSET) as *mut MidiEvent }
}

/// Returns pointers to the MIDI ring write/read atomics.
///
/// # Safety
/// `ptr` must point to a valid allocation containing the MIDI ring atomics.
pub unsafe fn midi_indices(ptr: *mut u8) -> (*mut AtomicU32, *mut AtomicU32) {
    unsafe {
        (
            ptr.add(MIDI_WRITE_IDX_OFFSET) as *mut AtomicU32,
            ptr.add(MIDI_READ_IDX_OFFSET) as *mut AtomicU32,
        )
    }
}

/// Returns a pointer to the echo ring buffer slot array.
///
/// # Safety
/// `ptr` must point to a valid allocation large enough to contain the echo ring.
pub unsafe fn echo_ring_ptr(ptr: *mut u8) -> *mut ParameterEvent {
    unsafe { ptr.add(ECHO_RING_OFFSET) as *mut ParameterEvent }
}

/// Returns pointers to the echo ring write/read atomics.
///
/// # Safety
/// `ptr` must point to a valid allocation containing the echo ring atomics.
pub unsafe fn echo_indices(ptr: *mut u8) -> (*mut AtomicU32, *mut AtomicU32) {
    unsafe {
        (
            ptr.add(ECHO_WRITE_IDX_OFFSET) as *mut AtomicU32,
            ptr.add(ECHO_READ_IDX_OFFSET) as *mut AtomicU32,
        )
    }
}

/// Returns a pointer to the MIDI output ring buffer slot array.
///
/// # Safety
/// `ptr` must point to a valid allocation large enough to contain the MIDI out ring.
pub unsafe fn midi_out_ring_ptr(ptr: *mut u8) -> *mut MidiEvent {
    unsafe { ptr.add(MIDI_OUT_RING_OFFSET) as *mut MidiEvent }
}

/// Returns pointers to the MIDI output ring write/read atomics.
///
/// # Safety
/// `ptr` must point to a valid allocation containing the MIDI out ring atomics.
pub unsafe fn midi_out_indices(ptr: *mut u8) -> (*mut AtomicU32, *mut AtomicU32) {
    unsafe {
        (
            ptr.add(MIDI_OUT_WRITE_IDX_OFFSET) as *mut AtomicU32,
            ptr.add(MIDI_OUT_READ_IDX_OFFSET) as *mut AtomicU32,
        )
    }
}

/// Returns a reference to the transport state.
///
/// # Safety
/// `ptr` must point to a valid allocation containing at least `TransportState`'s size.
pub unsafe fn transport_ref(ptr: *mut u8) -> &'static TransportState {
    unsafe { &*(ptr.add(TRANSPORT_OFFSET) as *mut TransportState) }
}

/// Returns a mutable reference to the transport state.
///
/// # Safety
/// `ptr` must point to a valid allocation containing at least `TransportState`'s size.
pub unsafe fn transport_mut(ptr: *mut u8) -> &'static mut TransportState {
    unsafe { &mut *(ptr.add(TRANSPORT_OFFSET) as *mut TransportState) }
}

/// Returns a pointer to the scratch buffer region.
///
/// # Safety
/// `ptr` must point to an allocation large enough to contain the scratch buffer.
pub unsafe fn scratch_ptr(ptr: *mut u8) -> *mut u8 {
    unsafe { ptr.add(SCRATCH_OFFSET) }
}

/// Write a plugin name to the start of the scratch buffer.
/// The name is encoded as a little-endian u32 length followed by UTF-8 bytes.
///
/// # Safety
/// `ptr` must point to a valid SHM allocation.
pub unsafe fn write_plugin_name_to_scratch(ptr: *mut u8, name: &str) {
    unsafe {
        let scratch = scratch_ptr(ptr);
        let bytes = name.as_bytes();
        let len = bytes.len().min(SCRATCH_SIZE - 4);
        std::ptr::write_unaligned(scratch as *mut u32, len as u32);
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), scratch.add(4), len);
    }
}

/// Read a plugin name from the start of the scratch buffer.
///
/// # Safety
/// `ptr` must point to a valid SHM allocation.
pub unsafe fn read_plugin_name_from_scratch(ptr: *mut u8) -> Option<String> {
    unsafe {
        let scratch = scratch_ptr(ptr);
        let len = std::ptr::read_unaligned(scratch as *mut u32) as usize;
        if len == 0 || len > SCRATCH_SIZE - 4 {
            return None;
        }
        let bytes = std::slice::from_raw_parts(scratch.add(4), len);
        String::from_utf8(bytes.to_vec()).ok()
    }
}

/// Magic value written before port counts in scratch.
pub const PORT_COUNTS_MAGIC: u32 = 0x504F_5254; // "PORT"

/// Offset within scratch where port counts are stored (after plugin name).
const PORT_COUNTS_OFFSET: usize = 1024;

/// Write audio/MIDI port counts to scratch.
///
/// # Safety
/// `ptr` must point to a valid SHM allocation.
pub unsafe fn write_port_counts_to_scratch(
    ptr: *mut u8,
    audio_in: u32,
    audio_out: u32,
    midi_in: u32,
    midi_out: u32,
) {
    unsafe {
        let dest = scratch_ptr(ptr).add(PORT_COUNTS_OFFSET);
        std::ptr::write_unaligned(dest as *mut u32, PORT_COUNTS_MAGIC);
        std::ptr::write_unaligned(dest.add(4) as *mut u32, audio_in);
        std::ptr::write_unaligned(dest.add(8) as *mut u32, audio_out);
        std::ptr::write_unaligned(dest.add(12) as *mut u32, midi_in);
        std::ptr::write_unaligned(dest.add(16) as *mut u32, midi_out);
    }
}

/// Read audio/MIDI port counts from scratch.
///
/// # Safety
/// `ptr` must point to a valid SHM allocation.
pub unsafe fn read_port_counts_from_scratch(ptr: *mut u8) -> Option<(u32, u32, u32, u32)> {
    unsafe {
        let src = scratch_ptr(ptr).add(PORT_COUNTS_OFFSET);
        let magic = std::ptr::read_unaligned(src as *mut u32);
        if magic != PORT_COUNTS_MAGIC {
            return None;
        }
        let audio_in = std::ptr::read_unaligned(src.add(4) as *mut u32);
        let audio_out = std::ptr::read_unaligned(src.add(8) as *mut u32);
        let midi_in = std::ptr::read_unaligned(src.add(12) as *mut u32);
        let midi_out = std::ptr::read_unaligned(src.add(16) as *mut u32);
        Some((audio_in, audio_out, midi_in, midi_out))
    }
}

// --- Static assertions for sizes ---

const _: () = assert!(std::mem::size_of::<ShmHeader>() == 256);
const _: () = assert!(std::mem::align_of::<ShmHeader>() == 256);
const _: () = assert!(std::mem::size_of::<ParameterEvent>() == 16);
const _: () = assert!(std::mem::align_of::<ParameterEvent>() == 16);
const _: () = assert!(std::mem::size_of::<MidiEvent>() == 16);
const _: () = assert!(std::mem::align_of::<MidiEvent>() == 16);
const _: () = assert!(std::mem::size_of::<TransportState>() == 256);
const _: () = assert!(std::mem::align_of::<TransportState>() == 256);
const _: () = assert!(LAYOUT_SIZE <= SHM_SIZE);

/// Wait (spin + yield) until `ready` becomes non-zero or timeout elapses.
pub fn wait_for_ready(header: &ShmHeader, timeout: std::time::Duration) -> bool {
    let start = std::time::Instant::now();
    while header.ready.load(Ordering::Acquire) == 0 {
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::yield_now();
    }
    true
}
