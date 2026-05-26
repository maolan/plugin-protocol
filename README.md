# maolan-plugin-protocol

Shared protocol types for Maolan out-of-process plugin hosting.

This crate defines the low-level IPC primitives used by the Maolan DAW to host
audio plugins (CLAP, VST3, LV2) in separate processes:

- **Shared-memory layout** (`shm`, `protocol`) – fixed-offset structs for
  audio buffers, parameter/MIDI ring buffers, transport state, and scratch space.
- **Lock-free ring buffers** (`ringbuf`) – single-producer / single-consumer
  queues backed by atomic indices.
- **Cross-process event pipes** (`events`) – Unix pipe pairs for bidirectional
  wake-up between the DAW and the plugin host process.

## Platform support

| Platform | Shared memory | Events |
|----------|---------------|--------|
| Unix     | ✅ POSIX `shm_open` / `mmap` | ✅ `pipe(2)` + `poll(2)` |
| Windows  | ✅ `CreateFileMappingW` / `MapViewOfFile` | ✅ Named auto-reset events (`CreateEventW` / `WaitForSingleObject`) |

## License

BSD-2-Clause
