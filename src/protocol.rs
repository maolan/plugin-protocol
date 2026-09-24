use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Magic number: "MAOL" in big-endian ASCII.
pub const MAGIC: u32 = 0x4D41_4F4C;

/// Current protocol version.
/// Version 2: parent_window changed from AtomicU32 to AtomicU64 to support 64-bit HWNDs on Windows.
/// Version 3: Added MIDI output ring for plugin-generated MIDI events.
/// Version 4: Per-port MIDI input/output rings (MAX_MIDI_PORTS each direction).
/// Version 5: Plugin-reported latency in samples.
/// Version 6: Added GUI parent API tag for native window handles.
/// Version 7: Replaced file-reference requests (enumerate/update) with
/// resource-directory requests (collect/enumerate); resource-directory
/// scratch payload now carries an `is_shared` flag.
///
/// Version 7, protocol crate 0.0.19: Two header fields were added inside
/// previously-padding bytes (offsets 88 and 92), so the on-the-wire header
/// layout is unchanged and binaries built against 0.0.18 still interoperate:
/// - `response_counter` (offset 88): bumped by the plugin-host before it
///   signals each completed audio block, letting the DAW poll block
///   completion in shared memory without a syscall.
/// - `block_response_eventless` (offset 92): set to 1 by the DAW when it will
///   observe the counter; the host then skips writing the per-block
///   completion event byte (which would otherwise accumulate in the pipe).
pub const VERSION: u32 = 7;

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
/// The atomics live at the start of the area, followed by 8 bytes of padding,
/// then the 16-byte-aligned `MidiEvent` slots.
pub const MIDI_PORT_RING_SIZE: usize = {
    let raw = 16 + MIDI_RING_SIZE; // head/tail atomics + padding + event slots
    (raw + 15) & !15 // align up to 16 bytes for the next port
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
pub const GUI_PARENT_API_OFFSET: usize = CONTROL_OFFSET + 20;

// --- Header field offsets ---
// The header is `#[repr(C, align(256))]`; offsets below match the field
// order in `ShmHeader`:
//   0  magic (u32)                4  version (u32)
//   8  flags (u32)               12 ready (AtomicU32)
//  16  heartbeat (AtomicU32)     20 error_code (u32)
//  24  shutdown_request          28 tasks_issued
//  32  tasks_completed           36 block_size
//  40  num_input_channels        44 num_output_channels
//  48  midi_in_port_count        52 midi_out_port_count
//  56  request_type              60 request_status
//  64  scratch_size              68..72 padding
//  72  parent_window (AtomicU64, 8 bytes)
//  80  state_dirty               84 latency_samples
//  88  response_counter (AtomicU32, added 0.0.19, was padding)
//  92  block_response_eventless (AtomicU32, added 0.0.19, was padding)
//  96..256 padding
/// Byte offset of the per-block response counter inside `ShmHeader`.
pub const RESPONSE_COUNTER_OFFSET: usize = 88;
/// Byte offset of the eventless block-response flag inside `ShmHeader`.
pub const BLOCK_RESPONSE_EVENTLESS_OFFSET: usize = 92;

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

/// Native window-system API for the GUI parent handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiParentApi {
    /// No native parent handle has been provided.
    #[default]
    None = 0,
    /// X11/Xlib window ID.
    X11 = 1,
    /// Wayland surface/object handle.
    Wayland = 2,
}

impl GuiParentApi {
    pub fn from_u32(value: u32) -> Self {
        match value {
            1 => GuiParentApi::X11,
            2 => GuiParentApi::Wayland,
            _ => GuiParentApi::None,
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
    /// 5 = set_resource_directory, 6 = collect_resources, 7 = enumerate_resource_files,
    /// 8 = enumerate_lv2_control_ports, 9 = enumerate_clap_parameters,
    /// 11 = enumerate_clap_note_names, 12 = enumerate_clap_audio_ports
    pub request_type: AtomicU32,
    /// Request status: 0 = pending, 1 = success, 2 = error
    pub request_status: AtomicU32,
    /// Valid bytes in scratch area for state operations
    pub scratch_size: AtomicU32,
    /// Parent window ID for GUI embedding. See GUI_PARENT_API_OFFSET for Unix API tagging.
    pub parent_window: AtomicU64,
    /// Set to 1 by the plugin-host when the plugin calls clap_host_state.mark_dirty()
    pub state_dirty: AtomicU32,
    /// Current plugin latency in samples, refreshed by the host.
    pub latency_samples: AtomicU32,
    /// Bumped by the plugin-host (Release) immediately before it reports each
    /// completed audio block, so the DAW can spin-poll block completion in
    /// shared memory instead of blocking on the event pipe. Previously part
    /// of `_pad`.
    pub response_counter: AtomicU32,
    /// Set to 1 by the DAW when it observes `response_counter` for block
    /// completion; the host then omits the per-block completion event byte
    /// (it would otherwise pile up unread in the pipe). Previously part of
    /// `_pad`.
    pub block_response_eventless: AtomicU32,
    _pad: [u8; 256 - 96],
}

impl ShmHeader {
    /// Bump the per-block response counter. Called by the plugin-host right
    /// before reporting a completed audio block (with the completion event,
    /// or alone when the DAW runs in eventless counter mode).
    pub fn mark_block_response(&self) {
        self.response_counter.fetch_add(1, Ordering::Release);
    }

    /// Current value of the per-block response counter.
    pub fn block_response_count(&self) -> u32 {
        self.response_counter.load(Ordering::Acquire)
    }

    /// Whether the DAW observes the response counter for block completion.
    pub fn block_response_eventless(&self) -> bool {
        self.block_response_eventless.load(Ordering::Acquire) != 0
    }

    /// Select whether the host should skip the per-block completion event
    /// byte because the DAW observes `response_counter` instead.
    pub fn set_block_response_eventless(&self, eventless: bool) {
        self.block_response_eventless
            .store(eventless as u32, Ordering::Release);
    }

    /// Load parent_window as a `usize` (handles 32- and 64-bit platforms).
    pub fn parent_window_usize(&self) -> usize {
        self.parent_window.load(Ordering::Acquire) as usize
    }

    /// Store a `usize` parent_window (truncates on 32-bit, but HWNDs/XIDs are
    /// always within 64 bits).
    pub fn set_parent_window(&self, window: usize) {
        self.parent_window.store(window as u64, Ordering::Release);
    }

    fn gui_parent_api_atomic(&self) -> &AtomicU32 {
        // SAFETY: GUI_PARENT_API_OFFSET is inside the control area, which is
        // within the header's 256-byte allocation. The offset is aligned to 4 bytes.
        unsafe {
            let base = self as *const Self as *const u8;
            &*(base.add(GUI_PARENT_API_OFFSET) as *const AtomicU32)
        }
    }

    /// Load the native API of the GUI parent handle.
    pub fn gui_parent_api(&self) -> GuiParentApi {
        GuiParentApi::from_u32(self.gui_parent_api_atomic().load(Ordering::Acquire))
    }

    /// Store the native API of the GUI parent handle.
    pub fn set_gui_parent_api(&self, api: GuiParentApi) {
        self.gui_parent_api_atomic()
            .store(api.as_u32(), Ordering::Release);
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
            latency_samples: AtomicU32::new(0),
            response_counter: AtomicU32::new(0),
            block_response_eventless: AtomicU32::new(0),
            _pad: [0; 256 - 96],
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
    unsafe { ptr.add(midi_port_ring_offset(MIDI_IN_RINGS_OFFSET, port) + 16) as *mut MidiEvent }
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
    unsafe { ptr.add(midi_port_ring_offset(MIDI_OUT_RINGS_OFFSET, port) + 16) as *mut MidiEvent }
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

/// Magic value written before the resource-file string list in scratch.
pub const FILE_REFS_MAGIC: u32 = 0x4649_4C45; // "FILE"

/// Offset within scratch where the resource-file string list is stored.
const FILE_REFS_OFFSET: usize = 2048;

/// Maximum total bytes available for the resource-file list.
const FILE_REFS_MAX_SIZE: usize = SCRATCH_SIZE - FILE_REFS_OFFSET;

/// A resource file used by a plugin in the shared resource folder, paired
/// with its plugin-side index.
pub type ResourceFile = (u32, String);

/// Write a list of resource files (index, relative path) to scratch.
/// Format: magic (u32), count (u32), then for each entry:
///   index (u32), length (u32) followed by UTF-8 bytes.
///
/// # Safety
/// `ptr` must point to a valid SHM allocation.
pub unsafe fn write_resource_files_to_scratch(
    ptr: *mut u8,
    files: &[ResourceFile],
) -> Result<(), String> {
    unsafe {
        let mut dest = scratch_ptr(ptr).add(FILE_REFS_OFFSET);
        let mut remaining = FILE_REFS_MAX_SIZE;
        if remaining < 8 {
            return Err("scratch too small for resource files".to_string());
        }
        std::ptr::write_unaligned(dest as *mut u32, FILE_REFS_MAGIC);
        dest = dest.add(4);
        remaining -= 4;
        let count = files.len().min(u32::MAX as usize) as u32;
        std::ptr::write_unaligned(dest as *mut u32, count);
        dest = dest.add(4);
        remaining -= 4;
        for (index, path) in files.iter().take(count as usize) {
            if remaining < 8 {
                return Err("scratch overflow writing resource files".to_string());
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
                return Err("scratch overflow writing resource files".to_string());
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

/// Read a list of resource files (index, relative path) from scratch.
///
/// # Safety
/// `ptr` must point to a valid SHM allocation.
pub unsafe fn read_resource_files_from_scratch(ptr: *mut u8) -> Option<Vec<ResourceFile>> {
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
        let mut files = Vec::with_capacity(count);
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
            files.push((index, path));
            src = src.add(len);
            remaining -= len;
        }
        Some(files)
    }
}

/// Write a resource-directory path and its sharing flag to scratch.
/// Format: magic (u32), length (u32), UTF-8 bytes, is_shared (u32).
///
/// # Safety
/// `ptr` must point to a valid SHM allocation.
pub unsafe fn write_resource_directory_to_scratch(
    ptr: *mut u8,
    path: &str,
    is_shared: bool,
) -> Result<(), String> {
    unsafe {
        let scratch = scratch_ptr(ptr);
        let bytes = path.as_bytes();
        let len = bytes.len().min(SCRATCH_SIZE - 12);
        if len < bytes.len() {
            return Err("resource directory path too long".to_string());
        }
        std::ptr::write_unaligned(scratch as *mut u32, FILE_REFS_MAGIC);
        std::ptr::write_unaligned(scratch.add(4) as *mut u32, len as u32);
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), scratch.add(8), len);
        std::ptr::write_unaligned(scratch.add(8 + len) as *mut u32, is_shared as u32);
        Ok(())
    }
}

/// Read a resource-directory path and its sharing flag from scratch.
///
/// # Safety
/// `ptr` must point to a valid SHM allocation.
pub unsafe fn read_resource_directory_from_scratch(ptr: *mut u8) -> Option<(String, bool)> {
    unsafe {
        let scratch = scratch_ptr(ptr);
        let magic = std::ptr::read_unaligned(scratch as *mut u32);
        if magic != FILE_REFS_MAGIC {
            return None;
        }
        let len = std::ptr::read_unaligned(scratch.add(4) as *mut u32) as usize;
        if len == 0 || len > SCRATCH_SIZE - 12 {
            return None;
        }
        let bytes = std::slice::from_raw_parts(scratch.add(8), len);
        let path = String::from_utf8(bytes.to_vec()).ok()?;
        let is_shared = std::ptr::read_unaligned(scratch.add(8 + len) as *mut u32) != 0;
        Some((path, is_shared))
    }
}

/// Request type: ask the plugin to copy its referenced resources into the
/// resource directory (`clap_plugin_resource_directory.collect`).
pub const REQUEST_COLLECT_RESOURCES: u32 = 6;

/// Request type: enumerate the files the plugin uses in the shared resource
/// folder (`clap_plugin_resource_directory.get_files_count/get_file_path`).
pub const REQUEST_RESOURCE_FILES: u32 = 7;

/// Request type: enumerate LV2 control ports (index, name, min, max, value).
pub const REQUEST_LV2_CONTROL_PORTS: u32 = 8;

/// Request type: enumerate CLAP parameters (id, name, module, min, max, default).
pub const REQUEST_CLAP_PARAMETERS: u32 = 9;

/// Request type: fetch LV2 midnam note names (MIDI note number -> name).
pub const REQUEST_LV2_MIDNAM: u32 = 10;

/// Request type: fetch CLAP note names (MIDI note number -> name).
pub const REQUEST_CLAP_NOTE_NAMES: u32 = 11;

/// Request type: refresh CLAP audio port counts in scratch.
pub const REQUEST_CLAP_AUDIO_PORTS: u32 = 12;

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

/// Returns a reference to the per-block response counter.
///
/// # Safety
/// `ptr` must point to a valid allocation containing at least `ShmHeader`'s size.
pub unsafe fn response_counter_ref(ptr: *mut u8) -> &'static std::sync::atomic::AtomicU32 {
    unsafe { &*(ptr.add(RESPONSE_COUNTER_OFFSET) as *const std::sync::atomic::AtomicU32) }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_counter_fields_live_at_offsets_88_and_92() {
        let header = ShmHeader::default();
        let base = &header as *const ShmHeader as usize;
        assert_eq!(
            &header.response_counter as *const AtomicU32 as usize - base,
            RESPONSE_COUNTER_OFFSET
        );
        assert_eq!(
            &header.block_response_eventless as *const AtomicU32 as usize - base,
            BLOCK_RESPONSE_EVENTLESS_OFFSET
        );
        // Both fields must sit inside the fixed 256-byte header, after
        // latency_samples (84) and clear of the parent_window u64 (72..80).
        const {
            assert!(RESPONSE_COUNTER_OFFSET >= 88);
            assert!(BLOCK_RESPONSE_EVENTLESS_OFFSET + 4 <= HEADER_SIZE);
        }
    }

    #[test]
    fn response_counter_handoff_roundtrip() {
        let mut bytes = vec![0u8; HEADER_SIZE];
        unsafe {
            init_shm_layout(bytes.as_mut_ptr(), bytes.len());
        }
        let header = unsafe { header_ref(bytes.as_mut_ptr()) };
        assert_eq!(header.block_response_count(), 0);
        assert!(!header.block_response_eventless());

        // Host side: two completed blocks, each marked before signalling.
        header.mark_block_response();
        header.mark_block_response();
        // DAW side: reads the counter through the raw-pointer helper, the
        // same view an engine built against 0.0.18 has of padding bytes
        // (zero, so its spin fast path simply never triggers).
        let counter = unsafe { response_counter_ref(bytes.as_mut_ptr()) };
        assert_eq!(counter.load(Ordering::Acquire), 2);
        assert_eq!(header.block_response_count(), 2);

        header.set_block_response_eventless(true);
        assert!(header.block_response_eventless());
    }
}
