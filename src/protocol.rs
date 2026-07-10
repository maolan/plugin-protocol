use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Magic number: "MAOL" in big-endian ASCII.
pub const MAGIC: u32 = 0x4D41_4F4C;

/// Current protocol version.
/// Version 2: parent_window changed from AtomicU32 to AtomicU64 to support 64-bit HWNDs on Windows.
/// Version 3: Added MIDI output ring for plugin-generated MIDI events.
/// Version 4: Per-port MIDI input/output rings (MAX_MIDI_PORTS each direction).
pub const VERSION: u32 = 4;

/// Maximum number of audio channels (main + sidechain combined).
pub const MAX_CHANNELS: usize = 32;

/// Number of audio buses (main + sidechain).
pub const NUM_BUSES: usize = 2;

/// Maximum audio block size in samples.
pub const MAX_BLOCK_SIZE: usize = 4096;

/// Capacity of each ring buffer in slots (power of two).
pub const RING_CAPACITY: usize = 4096;

/// Maximum number of MIDI ports per direction.
/// Runtime counts may be lower; this is the SHM capacity.
pub const MAX_MIDI_PORTS: usize = 16;

// --- Section sizes ---
pub const HEADER_SIZE: usize = 256;
pub const CONTROL_SIZE: usize = 256;
pub const AUDIO_BUFFER_SIZE: usize = MAX_CHANNELS * NUM_BUSES * MAX_BLOCK_SIZE * 4; // f32
pub const PARAM_RING_SIZE: usize = RING_CAPACITY * std::mem::size_of::<ParameterEvent>();
/// Size of the data area for one MIDI port ring (event slots only).
pub const MIDI_RING_SIZE: usize = RING_CAPACITY * std::mem::size_of::<MidiEvent>();
/// Size of one MIDI port ring area including embedded write/read atomics.
pub const MIDI_PORT_RING_SIZE: usize = {
    let raw = 8 + MIDI_RING_SIZE; // head + tail atomics + event slots
    (raw + 15) & !15 // align up to 16 bytes for MidiEvent
};
pub const TRANSPORT_SIZE: usize = 256;
pub const SCRATCH_SIZE: usize = 65536;

// --- Offsets into the shared-memory segment ---
/// Control area starts right after the header.
pub const CONTROL_OFFSET: usize = HEADER_SIZE;
/// Audio buffers start after the control area.
pub const AUDIO_OFFSET: usize = HEADER_SIZE + CONTROL_SIZE;
/// Parameter ring buffer.
pub const PARAM_RING_OFFSET: usize = AUDIO_OFFSET + AUDIO_BUFFER_SIZE;
/// Echo/parameter-change ring buffer.
pub const ECHO_RING_OFFSET: usize = PARAM_RING_OFFSET + PARAM_RING_SIZE;
pub const ECHO_RING_SIZE: usize = RING_CAPACITY * std::mem::size_of::<ParameterEvent>();
/// Per-port MIDI input rings start after the echo ring.
pub const MIDI_IN_RINGS_OFFSET: usize = {
    let end = ECHO_RING_OFFSET + ECHO_RING_SIZE;
    (end + 255) & !255
};
pub const MIDI_IN_RINGS_SIZE: usize = MAX_MIDI_PORTS * MIDI_PORT_RING_SIZE;
/// Per-port MIDI output rings follow the input rings.
pub const MIDI_OUT_RINGS_OFFSET: usize = MIDI_IN_RINGS_OFFSET + MIDI_IN_RINGS_SIZE;
pub const MIDI_OUT_RINGS_SIZE: usize = MAX_MIDI_PORTS * MIDI_PORT_RING_SIZE;
/// Transport state block (256-byte aligned from here).
pub const TRANSPORT_OFFSET: usize = {
    let end = MIDI_OUT_RINGS_OFFSET + MIDI_OUT_RINGS_SIZE;
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
pub const ECHO_WRITE_IDX_OFFSET: usize = CONTROL_OFFSET + 8;
pub const ECHO_READ_IDX_OFFSET: usize = CONTROL_OFFSET + 12;
pub const GUI_MODE_OFFSET: usize = CONTROL_OFFSET + 16;

/// GUI mode requested by the DAW.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiMode {
    /// DAW provides a parent window; plugin UI should be embedded.
    #[default]
    Embedded = 0,
    /// DAW cannot provide a parent window; plugin-host must create a top-level window.
    Floating = 1,
}

impl GuiMode {
    pub fn from_u32(value: u32) -> Self {
        match value {
            1 => GuiMode::Floating,
            _ => GuiMode::Embedded,
        }
    }

    pub fn as_u32(self) -> u32 {
        self as u32
    }
}

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
    /// Number of MIDI input ports actually used by the plugin (<= MAX_MIDI_PORTS).
    pub midi_in_port_count: AtomicU32,
    /// Number of MIDI output ports actually used by the plugin (<= MAX_MIDI_PORTS).
    pub midi_out_port_count: AtomicU32,
    /// Request type: 0 = none, 1 = save_state, 2 = restore_state, 3 = gui_show, 4 = gui_hide,
    /// 5 = set_resource_directory, 6 = enumerate_file_references, 7 = update_file_reference,
    /// 8 = enumerate_lv2_control_ports, 9 = enumerate_clap_parameters
    pub request_type: AtomicU32,
    /// Request status: 0 = pending, 1 = success, 2 = error
    pub request_status: AtomicU32,
    /// Valid bytes in scratch area for state operations
    pub scratch_size: AtomicU32,
    /// Parent window ID for GUI embedding (X11 window ID on Unix, HWND on Windows)
    pub parent_window: AtomicU64,
    /// Set to 1 by the plugin-host when the plugin calls clap_host_state.mark_dirty()
    pub state_dirty: AtomicU32,
    _pad: [u8; 256 - 84],
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

    fn gui_mode_atomic(&self) -> &AtomicU32 {
        // SAFETY: GUI_MODE_OFFSET is inside the control area, which is within the
        // header's 256-byte allocation. The offset is aligned to 4 bytes.
        unsafe {
            let base = self as *const Self as *const u8;
            &*(base.add(GUI_MODE_OFFSET) as *const AtomicU32)
        }
    }

    /// Load the requested GUI mode.
    pub fn gui_mode(&self) -> GuiMode {
        GuiMode::from_u32(self.gui_mode_atomic().load(Ordering::Acquire))
    }

    /// Store the requested GUI mode.
    pub fn set_gui_mode(&self, mode: GuiMode) {
        self.gui_mode_atomic()
            .store(mode.as_u32(), Ordering::Release);
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
            midi_in_port_count: AtomicU32::new(0),
            midi_out_port_count: AtomicU32::new(0),
            request_type: AtomicU32::new(0),
            request_status: AtomicU32::new(0),
            scratch_size: AtomicU32::new(0),
            parent_window: AtomicU64::new(0),
            state_dirty: AtomicU32::new(0),
            _pad: [0; 256 - 84],
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

const fn midi_port_ring_offset(base_offset: usize, port: usize) -> usize {
    base_offset + port * MIDI_PORT_RING_SIZE
}

/// Returns pointers to the embedded write/read atomics for a MIDI input port ring.
///
/// # Safety
/// `ptr` must point to a valid allocation and `port` must be < MAX_MIDI_PORTS.
pub unsafe fn midi_in_indices(ptr: *mut u8, port: usize) -> (*mut AtomicU32, *mut AtomicU32) {
    unsafe {
        let base = ptr.add(midi_port_ring_offset(MIDI_IN_RINGS_OFFSET, port));
        (base as *mut AtomicU32, base.add(4) as *mut AtomicU32)
    }
}

/// Returns a pointer to the MIDI input port ring buffer slot array.
///
/// # Safety
/// `ptr` must point to a valid allocation and `port` must be < MAX_MIDI_PORTS.
pub unsafe fn midi_in_ring_ptr(ptr: *mut u8, port: usize) -> *mut MidiEvent {
    unsafe { ptr.add(midi_port_ring_offset(MIDI_IN_RINGS_OFFSET, port) + 8) as *mut MidiEvent }
}

/// Returns pointers to the embedded write/read atomics for a MIDI output port ring.
///
/// # Safety
/// `ptr` must point to a valid allocation and `port` must be < MAX_MIDI_PORTS.
pub unsafe fn midi_out_indices(ptr: *mut u8, port: usize) -> (*mut AtomicU32, *mut AtomicU32) {
    unsafe {
        let base = ptr.add(midi_port_ring_offset(MIDI_OUT_RINGS_OFFSET, port));
        (base as *mut AtomicU32, base.add(4) as *mut AtomicU32)
    }
}

/// Returns a pointer to the MIDI output port ring buffer slot array.
///
/// # Safety
/// `ptr` must point to a valid allocation and `port` must be < MAX_MIDI_PORTS.
pub unsafe fn midi_out_ring_ptr(ptr: *mut u8, port: usize) -> *mut MidiEvent {
    unsafe { ptr.add(midi_port_ring_offset(MIDI_OUT_RINGS_OFFSET, port) + 8) as *mut MidiEvent }
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

/// Magic value written before file-reference string list in scratch.
pub const FILE_REFS_MAGIC: u32 = 0x4649_4C45; // "FILE"

/// Offset within scratch where file-reference string list is stored.
const FILE_REFS_OFFSET: usize = 2048;

/// Maximum total bytes available for the file-reference list.
const FILE_REFS_MAX_SIZE: usize = SCRATCH_SIZE - FILE_REFS_OFFSET;

/// A file reference returned by a plugin, paired with its plugin-side index.
pub type FileReference = (u32, String);

/// Write a list of file-reference (index, path) pairs to scratch.
/// Format: magic (u32), count (u32), then for each entry:
///   index (u32), length (u32) followed by UTF-8 bytes.
///
/// # Safety
/// `ptr` must point to a valid SHM allocation.
pub unsafe fn write_file_references_to_scratch(
    ptr: *mut u8,
    refs: &[FileReference],
) -> Result<(), String> {
    unsafe {
        let mut dest = scratch_ptr(ptr).add(FILE_REFS_OFFSET);
        let mut remaining = FILE_REFS_MAX_SIZE;
        if remaining < 8 {
            return Err("scratch too small for file references".to_string());
        }
        std::ptr::write_unaligned(dest as *mut u32, FILE_REFS_MAGIC);
        dest = dest.add(4);
        remaining -= 4;
        let count = refs.len().min(u32::MAX as usize) as u32;
        std::ptr::write_unaligned(dest as *mut u32, count);
        dest = dest.add(4);
        remaining -= 4;
        for (index, path) in refs.iter().take(count as usize) {
            if remaining < 8 {
                return Err("scratch overflow writing file references".to_string());
            }
            std::ptr::write_unaligned(dest as *mut u32, *index);
            dest = dest.add(4);
            remaining -= 4;
            let bytes = path.as_bytes();
            let len = bytes
                .len()
                .min(u32::MAX as usize)
                .min(remaining.saturating_sub(4));
            if len < bytes.len() {
                return Err("scratch overflow writing file references".to_string());
            }
            std::ptr::write_unaligned(dest as *mut u32, len as u32);
            dest = dest.add(4);
            remaining -= 4;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), dest, len);
            dest = dest.add(len);
            remaining -= len;
        }
        Ok(())
    }
}

/// Read a list of file-reference (index, path) pairs from scratch.
///
/// # Safety
/// `ptr` must point to a valid SHM allocation.
pub unsafe fn read_file_references_from_scratch(ptr: *mut u8) -> Option<Vec<FileReference>> {
    unsafe {
        let mut src = scratch_ptr(ptr).add(FILE_REFS_OFFSET);
        let mut remaining = FILE_REFS_MAX_SIZE;
        if remaining < 8 {
            return None;
        }
        let magic = std::ptr::read_unaligned(src as *mut u32);
        if magic != FILE_REFS_MAGIC {
            return None;
        }
        src = src.add(4);
        remaining -= 4;
        let count = std::ptr::read_unaligned(src as *mut u32) as usize;
        src = src.add(4);
        remaining -= 4;
        let mut refs = Vec::with_capacity(count);
        for _ in 0..count {
            if remaining < 8 {
                return None;
            }
            let index = std::ptr::read_unaligned(src as *mut u32);
            src = src.add(4);
            remaining -= 4;
            let len = std::ptr::read_unaligned(src as *mut u32) as usize;
            src = src.add(4);
            remaining -= 4;
            if len > remaining {
                return None;
            }
            let bytes = std::slice::from_raw_parts(src, len);
            let path = String::from_utf8(bytes.to_vec()).ok()?;
            refs.push((index, path));
            src = src.add(len);
            remaining -= len;
        }
        Some(refs)
    }
}

/// Write a resource-directory / base-directory path to scratch.
/// Format: magic (u32), length (u32), UTF-8 bytes.
///
/// # Safety
/// `ptr` must point to a valid SHM allocation.
pub unsafe fn write_resource_directory_to_scratch(ptr: *mut u8, path: &str) -> Result<(), String> {
    unsafe {
        let scratch = scratch_ptr(ptr);
        let bytes = path.as_bytes();
        let len = bytes.len().min(SCRATCH_SIZE - 8);
        if len < bytes.len() {
            return Err("resource directory path too long".to_string());
        }
        std::ptr::write_unaligned(scratch as *mut u32, FILE_REFS_MAGIC);
        std::ptr::write_unaligned(scratch.add(4) as *mut u32, len as u32);
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), scratch.add(8), len);
        Ok(())
    }
}

/// Read a resource-directory / base-directory path from scratch.
///
/// # Safety
/// `ptr` must point to a valid SHM allocation.
pub unsafe fn read_resource_directory_from_scratch(ptr: *mut u8) -> Option<String> {
    unsafe {
        let scratch = scratch_ptr(ptr);
        let magic = std::ptr::read_unaligned(scratch as *mut u32);
        if magic != FILE_REFS_MAGIC {
            return None;
        }
        let len = std::ptr::read_unaligned(scratch.add(4) as *mut u32) as usize;
        if len == 0 || len > SCRATCH_SIZE - 8 {
            return None;
        }
        let bytes = std::slice::from_raw_parts(scratch.add(8), len);
        String::from_utf8(bytes.to_vec()).ok()
    }
}

/// Request type: enumerate LV2 control ports (index, name, min, max, value).
pub const REQUEST_LV2_CONTROL_PORTS: u32 = 8;

/// Request type: enumerate CLAP parameters (id, name, module, min, max, default).
pub const REQUEST_CLAP_PARAMETERS: u32 = 9;

/// Request type: fetch LV2 midnam note names (MIDI note number -> name).
pub const REQUEST_LV2_MIDNAM: u32 = 10;

/// Request type: fetch CLAP note names (MIDI note number -> name).
pub const REQUEST_CLAP_NOTE_NAMES: u32 = 11;

/// Magic value for a single file-reference update in scratch.
pub const FILE_REF_UPDATE_MAGIC: u32 = 0x5550_4441; // "UPDA"

/// Write a file-reference update (index + new path) to scratch.
/// Format: magic (u32), index (u32), length (u32), UTF-8 bytes.
///
/// # Safety
/// `ptr` must point to a valid SHM allocation.
pub unsafe fn write_file_reference_update_to_scratch(
    ptr: *mut u8,
    index: u32,
    path: &str,
) -> Result<(), String> {
    unsafe {
        let scratch = scratch_ptr(ptr);
        let bytes = path.as_bytes();
        let len = bytes.len().min(SCRATCH_SIZE - 12);
        if len < bytes.len() {
            return Err("file-reference update path too long".to_string());
        }
        std::ptr::write_unaligned(scratch as *mut u32, FILE_REF_UPDATE_MAGIC);
        std::ptr::write_unaligned(scratch.add(4) as *mut u32, index);
        std::ptr::write_unaligned(scratch.add(8) as *mut u32, len as u32);
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), scratch.add(12), len);
        Ok(())
    }
}

/// Read a file-reference update (index + new path) from scratch.
///
/// # Safety
/// `ptr` must point to a valid SHM allocation.
pub unsafe fn read_file_reference_update_from_scratch(ptr: *mut u8) -> Option<(u32, String)> {
    unsafe {
        let scratch = scratch_ptr(ptr);
        let magic = std::ptr::read_unaligned(scratch as *mut u32);
        if magic != FILE_REF_UPDATE_MAGIC {
            return None;
        }
        let index = std::ptr::read_unaligned(scratch.add(4) as *mut u32);
        let len = std::ptr::read_unaligned(scratch.add(8) as *mut u32) as usize;
        if len == 0 || len > SCRATCH_SIZE - 12 {
            return None;
        }
        let bytes = std::slice::from_raw_parts(scratch.add(12), len);
        let path = String::from_utf8(bytes.to_vec()).ok()?;
        Some((index, path))
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
