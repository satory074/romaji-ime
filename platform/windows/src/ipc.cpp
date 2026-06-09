#include "ipc.h"

namespace romaji {
namespace {

// ---- bincode encoders (little-endian, fixed width) -----------------------

void PutU32(std::vector<uint8_t>& b, uint32_t v) {
    for (int i = 0; i < 4; ++i) b.push_back(static_cast<uint8_t>((v >> (8 * i)) & 0xFF));
}
void PutU64(std::vector<uint8_t>& b, uint64_t v) {
    for (int i = 0; i < 8; ++i) b.push_back(static_cast<uint8_t>((v >> (8 * i)) & 0xFF));
}
void PutString(std::vector<uint8_t>& b, const std::string& s) {
    PutU64(b, s.size());
    b.insert(b.end(), s.begin(), s.end());
}

// ---- bincode decoder cursor ----------------------------------------------

struct Reader {
    const uint8_t* p;
    const uint8_t* end;
    bool ok = true;

    uint32_t U32() {
        if (p + 4 > end) { ok = false; return 0; }
        uint32_t v = 0;
        for (int i = 0; i < 4; ++i) v |= static_cast<uint32_t>(*p++) << (8 * i);
        return v;
    }
    uint64_t U64() {
        if (p + 8 > end) { ok = false; return 0; }
        uint64_t v = 0;
        for (int i = 0; i < 8; ++i) v |= static_cast<uint64_t>(*p++) << (8 * i);
        return v;
    }
    std::string Utf8() {
        uint64_t len = U64();
        if (!ok || p + len > end) { ok = false; return {}; }
        std::string s(reinterpret_cast<const char*>(p), static_cast<size_t>(len));
        p += len;
        return s;
    }
};

std::wstring Utf8ToWide(const std::string& s) {
    if (s.empty()) return {};
    int n = MultiByteToWideChar(CP_UTF8, 0, s.data(), static_cast<int>(s.size()), nullptr, 0);
    std::wstring w(n, L'\0');
    MultiByteToWideChar(CP_UTF8, 0, s.data(), static_cast<int>(s.size()), w.data(), n);
    return w;
}

std::string WideToUtf8(const std::wstring& w) {
    if (w.empty()) return {};
    int n = WideCharToMultiByte(CP_UTF8, 0, w.data(), static_cast<int>(w.size()), nullptr, 0,
                                nullptr, nullptr);
    std::string s(n, '\0');
    WideCharToMultiByte(CP_UTF8, 0, w.data(), static_cast<int>(w.size()), s.data(), n, nullptr,
                        nullptr);
    return s;
}

// Response variant tags (docs/ipc-protocol.md).
enum class RespTag : uint32_t {
    SessionId = 0,
    State = 1,
    AiBegun = 2,
    Pending = 3,
    Ok = 4,
    Error = 5,
};

// Request variant tags.
constexpr uint32_t kReqNewSession = 0;
constexpr uint32_t kReqCloseSession = 1;
constexpr uint32_t kReqProcessKey = 2;
constexpr uint32_t kReqReset = 4;
constexpr uint32_t kReqBeginAiConvert = 5;
constexpr uint32_t kReqPollAiResult = 6;

State DecodeState(Reader& r) {
    State st;
    st.flags = r.U32();
    st.preedit = Utf8ToWide(r.Utf8());
    st.commit = Utf8ToWide(r.Utf8());
    uint64_t count = r.U64();
    for (uint64_t i = 0; i < count && r.ok; ++i) {
        st.candidates.push_back(Utf8ToWide(r.Utf8()));
    }
    st.highlighted = r.U64();
    return st;
}

constexpr uint32_t kMaxFrame = 16u * 1024u * 1024u;

}  // namespace

PipeClient::PipeClient(std::wstring pipe_name) : pipe_name_(std::move(pipe_name)) {}

PipeClient::~PipeClient() { Disconnect(); }

void PipeClient::Disconnect() {
    if (handle_ != INVALID_HANDLE_VALUE) {
        CloseHandle(handle_);
        handle_ = INVALID_HANDLE_VALUE;
    }
}

bool PipeClient::EnsureConnected() {
    if (handle_ != INVALID_HANDLE_VALUE) return true;
    // The server may be launching; one short wait is enough for the common case.
    handle_ = CreateFileW(pipe_name_.c_str(), GENERIC_READ | GENERIC_WRITE, 0, nullptr,
                          OPEN_EXISTING, 0, nullptr);
    if (handle_ == INVALID_HANDLE_VALUE && GetLastError() == ERROR_PIPE_BUSY) {
        if (WaitNamedPipeW(pipe_name_.c_str(), 200)) {
            handle_ = CreateFileW(pipe_name_.c_str(), GENERIC_READ | GENERIC_WRITE, 0, nullptr,
                                  OPEN_EXISTING, 0, nullptr);
        }
    }
    return handle_ != INVALID_HANDLE_VALUE;
}

bool PipeClient::SendFrame(const std::vector<uint8_t>& payload) {
    uint8_t len[4];
    uint32_t n = static_cast<uint32_t>(payload.size());
    for (int i = 0; i < 4; ++i) len[i] = static_cast<uint8_t>((n >> (8 * i)) & 0xFF);
    DWORD wrote = 0;
    if (!WriteFile(handle_, len, 4, &wrote, nullptr) || wrote != 4) return false;
    if (!payload.empty()) {
        if (!WriteFile(handle_, payload.data(), n, &wrote, nullptr) || wrote != n) return false;
    }
    return true;
}

static bool ReadExact(HANDLE h, uint8_t* buf, uint32_t n) {
    uint32_t got = 0;
    while (got < n) {
        DWORD r = 0;
        if (!ReadFile(h, buf + got, n - got, &r, nullptr) || r == 0) return false;
        got += r;
    }
    return true;
}

bool PipeClient::RecvFrame(std::vector<uint8_t>& out) {
    uint8_t len[4];
    if (!ReadExact(handle_, len, 4)) return false;
    uint32_t n = 0;
    for (int i = 0; i < 4; ++i) n |= static_cast<uint32_t>(len[i]) << (8 * i);
    if (n > kMaxFrame) return false;
    out.resize(n);
    if (n > 0 && !ReadExact(handle_, out.data(), n)) return false;
    return true;
}

std::optional<std::vector<uint8_t>> PipeClient::Call(const std::vector<uint8_t>& request) {
    if (!EnsureConnected()) return std::nullopt;
    if (!SendFrame(request)) {
        Disconnect();  // stale handle; caller may retry
        return std::nullopt;
    }
    std::vector<uint8_t> resp;
    if (!RecvFrame(resp)) {
        Disconnect();
        return std::nullopt;
    }
    return resp;
}

std::optional<uint64_t> PipeClient::NewSession() {
    std::vector<uint8_t> req;
    PutU32(req, kReqNewSession);
    auto resp = Call(req);
    if (!resp) return std::nullopt;
    Reader r{resp->data(), resp->data() + resp->size()};
    if (static_cast<RespTag>(r.U32()) != RespTag::SessionId || !r.ok) return std::nullopt;
    uint64_t sid = r.U64();
    return r.ok ? std::optional<uint64_t>(sid) : std::nullopt;
}

std::optional<State> PipeClient::ProcessKey(uint64_t sid, uint32_t keysym, uint32_t mods) {
    std::vector<uint8_t> req;
    PutU32(req, kReqProcessKey);
    PutU64(req, sid);
    PutU32(req, keysym);
    PutU32(req, mods);
    auto resp = Call(req);
    if (!resp) return std::nullopt;
    Reader r{resp->data(), resp->data() + resp->size()};
    if (static_cast<RespTag>(r.U32()) != RespTag::State || !r.ok) return std::nullopt;
    State st = DecodeState(r);
    return r.ok ? std::optional<State>(std::move(st)) : std::nullopt;
}

std::optional<State> PipeClient::Reset(uint64_t sid) {
    std::vector<uint8_t> req;
    PutU32(req, kReqReset);
    PutU64(req, sid);
    auto resp = Call(req);
    if (!resp) return std::nullopt;
    Reader r{resp->data(), resp->data() + resp->size()};
    if (static_cast<RespTag>(r.U32()) != RespTag::State || !r.ok) return std::nullopt;
    State st = DecodeState(r);
    return r.ok ? std::optional<State>(std::move(st)) : std::nullopt;
}

void PipeClient::CloseSession(uint64_t sid) {
    std::vector<uint8_t> req;
    PutU32(req, kReqCloseSession);
    PutU64(req, sid);
    (void)Call(req);
}

std::optional<uint64_t> PipeClient::BeginAiConvert(uint64_t sid, const std::wstring& before,
                                                   const std::wstring& after) {
    std::vector<uint8_t> req;
    PutU32(req, kReqBeginAiConvert);
    PutU64(req, sid);
    PutString(req, WideToUtf8(before));
    PutString(req, WideToUtf8(after));
    auto resp = Call(req);
    if (!resp) return std::nullopt;
    Reader r{resp->data(), resp->data() + resp->size()};
    if (static_cast<RespTag>(r.U32()) != RespTag::AiBegun || !r.ok) return std::nullopt;
    uint64_t id = r.U64();
    return r.ok ? std::optional<uint64_t>(id) : std::nullopt;
}

PipeClient::PollOutcome PipeClient::PollAiResult(uint64_t sid, uint64_t req_id) {
    PollOutcome out;
    std::vector<uint8_t> req;
    PutU32(req, kReqPollAiResult);
    PutU64(req, sid);
    PutU64(req, req_id);
    auto resp = Call(req);
    if (!resp) {
        out.kind = PollKind::Error;
        return out;
    }
    Reader r{resp->data(), resp->data() + resp->size()};
    RespTag tag = static_cast<RespTag>(r.U32());
    if (!r.ok) {
        out.kind = PollKind::Error;
        return out;
    }
    switch (tag) {
        case RespTag::Pending:
            out.kind = PollKind::Pending;
            break;
        case RespTag::State:
            out.state = DecodeState(r);
            out.kind = r.ok ? PollKind::Ready : PollKind::Error;
            break;
        default:  // Error / unexpected
            out.kind = PollKind::Error;
            break;
    }
    return out;
}

}  // namespace romaji
