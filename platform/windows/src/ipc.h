// Named-pipe client + bincode codec for talking to ime-server.
//
// This mirrors the wire format pinned in docs/ipc-protocol.md and the Rust
// `ime-ipc` crate. If the Rust byte-layout test changes, update both.
#pragma once

#include <windows.h>

#include <cstdint>
#include <optional>
#include <string>
#include <vector>

namespace romaji {

// Result-flag bits (must match ime_engine::flags / romaji_ime.h).
constexpr uint32_t kFlagConsumed = 1;
constexpr uint32_t kFlagPreedit = 2;
constexpr uint32_t kFlagCandidates = 4;
constexpr uint32_t kFlagCommit = 8;

// Mirrors ime_ipc::State (strings decoded to UTF-16 for Windows use).
struct State {
    uint32_t flags = 0;
    std::wstring preedit;
    std::wstring commit;
    std::vector<std::wstring> candidates;
    uint64_t highlighted = 0;
};

// A thin client over the ime-server named pipe. Reconnects on demand; all calls
// fail soft (return nullopt) so the TSF DLL can fall back rather than hang.
class PipeClient {
public:
    explicit PipeClient(std::wstring pipe_name);
    ~PipeClient();

    PipeClient(const PipeClient&) = delete;
    PipeClient& operator=(const PipeClient&) = delete;

    std::optional<uint64_t> NewSession();
    std::optional<State> ProcessKey(uint64_t sid, uint32_t keysym, uint32_t mods);
    std::optional<State> Reset(uint64_t sid);
    void CloseSession(uint64_t sid);

    // Cloud-AI conversion. Begin returns a request id (or nullopt if
    // unavailable); poll it until Ready/Error. Run these OFF the UI thread (the
    // server does a slow LLM call) — see platform/windows/README.md.
    // explicit_ = true: Space-triggered convert (engages candidate selection so
    // Enter commits the chosen candidate). false: typing-pause auto-convert
    // (non-committal preview; Enter commits the raw text until Space engages it).
    std::optional<uint64_t> BeginAiConvert(uint64_t sid, const std::wstring& context_before,
                                           const std::wstring& context_after, bool explicit_);

    enum class PollKind { Pending, Ready, Error };
    struct PollOutcome {
        PollKind kind = PollKind::Error;
        State state;  // valid when kind == Ready
    };
    PollOutcome PollAiResult(uint64_t sid, uint64_t req_id);

private:
    bool EnsureConnected();
    void Disconnect();
    bool SendFrame(const std::vector<uint8_t>& payload);
    bool RecvFrame(std::vector<uint8_t>& out);
    // Send one request payload and return the parsed response payload bytes.
    std::optional<std::vector<uint8_t>> Call(const std::vector<uint8_t>& request);

    std::wstring pipe_name_;
    HANDLE handle_ = INVALID_HANDLE_VALUE;
};

}  // namespace romaji
