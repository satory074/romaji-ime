# IPC protocol (Windows DLL ⇄ ime-server)

Authoritative wire-format spec for the named pipe between the thin TSF DLL
(`platform/windows`) and `ime-server` (`crates/ime-server`). The Rust side is
defined in `crates/ime-ipc`; the C++ side hand-implements this same format in
`platform/windows/src/ipc.*`. The Rust test
`ime_ipc::tests::process_key_byte_layout_is_stable` pins the byte layout — if it
changes, update this doc **and** the C++ codec together.

## Framing

Every message is:

```
[u32 length, little-endian] [ length bytes of bincode payload ]
```

`length` must be ≤ `MAX_FRAME_LEN` (16 MiB). One request → one response,
sequential per connection.

## Payload encoding (bincode, default config)

- Integers: fixed width, little-endian (`u32` = 4 bytes, `u64` = 8 bytes).
- `enum`: a `u32` little-endian **variant index**, then the variant's fields in
  declaration order.
- `String`: `u64` little-endian byte length, then that many UTF-8 bytes.
- `Vec<T>`: `u64` little-endian element count, then each element encoded in turn.
- `bool`: a single byte, `0x00` (false) or `0x01` (true).
- `struct`: fields in declaration order, no tag.

## Request (variant indices)

| idx | variant | fields (in order) |
|----:|---------|-------------------|
| 0 | `NewSession` | — |
| 1 | `CloseSession` | `sid: u64` |
| 2 | `ProcessKey` | `sid: u64`, `keysym: u32`, `mods: u32` |
| 3 | `SelectCandidate` | `sid: u64`, `index: u64` |
| 4 | `Reset` | `sid: u64` |
| 5 | `BeginAiConvert` | `sid: u64`, `context_before: String`, `context_after: String`, `explicit: bool` |
| 6 | `PollAiResult` | `sid: u64`, `req_id: u64` |

## Response (variant indices)

| idx | variant | fields |
|----:|---------|--------|
| 0 | `SessionId` | `sid: u64` |
| 1 | `State` | `State` (one field, the struct below) |
| 2 | `AiBegun` | `req_id: u64` |
| 3 | `Pending` | — |
| 4 | `Ok` | — |
| 5 | `Error` | `message: String` |

### `State` struct (fields in order)

| field | type |
|-------|------|
| `flags` | `u32` (CONSUMED=1, PREEDIT=2, CANDIDATES=4, COMMIT=8) |
| `preedit` | `String` |
| `commit` | `String` |
| `candidates` | `Vec<String>` |
| `highlighted` | `u64` |

## Worked example

`Request::ProcessKey { sid: 1, keysym: 0x61, mods: 0 }` encodes (20 bytes), then
framed with its length prefix (24 bytes total):

```
24 00 00 00                 # frame length = 20
02 00 00 00                 # variant 2 (ProcessKey)
01 00 00 00 00 00 00 00     # sid = 1
61 00 00 00                 # keysym = 0x61
00 00 00 00                 # mods = 0
```
