#include "TextService.h"

#include <string>

// ---------------------------------------------------------------------------
// Edit session that applies one engine State to the document. TSF requires all
// document mutation (composition, selection) to happen inside an edit session.
// ---------------------------------------------------------------------------
namespace {
class CApplyStateEditSession final : public ITfEditSession {
public:
    CApplyStateEditSession(CTextService* pTS, ITfContext* pContext, romaji::State state)
        : _cRef(1), _pTS(pTS), _pContext(pContext), _state(std::move(state)) {
        _pTS->AddRef();
        _pContext->AddRef();
    }

    STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override {
        if (!ppv) return E_INVALIDARG;
        if (IsEqualIID(riid, IID_IUnknown) || IsEqualIID(riid, IID_ITfEditSession)) {
            *ppv = static_cast<ITfEditSession*>(this);
            AddRef();
            return S_OK;
        }
        *ppv = nullptr;
        return E_NOINTERFACE;
    }
    STDMETHODIMP_(ULONG) AddRef() override { return ++_cRef; }
    STDMETHODIMP_(ULONG) Release() override {
        LONG c = --_cRef;
        if (c == 0) delete this;
        return c;
    }
    STDMETHODIMP DoEditSession(TfEditCookie ec) override {
        _pTS->ApplyStateInEditSession(ec, _pContext, _state);
        return S_OK;
    }

private:
    ~CApplyStateEditSession() {
        _pContext->Release();
        _pTS->Release();
    }
    LONG _cRef;
    CTextService* _pTS;
    ITfContext* _pContext;
    romaji::State _state;
};
}  // namespace

// ---------------------------------------------------------------------------
// Construction / IUnknown
// ---------------------------------------------------------------------------

CTextService::CTextService() : _cRef(1) { DllAddRef(); }
CTextService::~CTextService() { DllRelease(); }

STDMETHODIMP CTextService::QueryInterface(REFIID riid, void** ppv) {
    if (!ppv) return E_INVALIDARG;
    if (IsEqualIID(riid, IID_IUnknown) || IsEqualIID(riid, IID_ITfTextInputProcessor)) {
        *ppv = static_cast<ITfTextInputProcessor*>(this);
    } else if (IsEqualIID(riid, IID_ITfTextInputProcessorEx)) {
        *ppv = static_cast<ITfTextInputProcessorEx*>(this);
    } else if (IsEqualIID(riid, IID_ITfThreadMgrEventSink)) {
        *ppv = static_cast<ITfThreadMgrEventSink*>(this);
    } else if (IsEqualIID(riid, IID_ITfKeyEventSink)) {
        *ppv = static_cast<ITfKeyEventSink*>(this);
    } else if (IsEqualIID(riid, IID_ITfCompositionSink)) {
        *ppv = static_cast<ITfCompositionSink*>(this);
    } else {
        *ppv = nullptr;
        return E_NOINTERFACE;
    }
    AddRef();
    return S_OK;
}

STDMETHODIMP_(ULONG) CTextService::AddRef() { return InterlockedIncrement(&_cRef); }

STDMETHODIMP_(ULONG) CTextService::Release() {
    LONG c = InterlockedDecrement(&_cRef);
    if (c == 0) delete this;
    return c;
}

// ---------------------------------------------------------------------------
// Activation
// ---------------------------------------------------------------------------

STDMETHODIMP CTextService::Activate(ITfThreadMgr* ptim, TfClientId tid) {
    return ActivateEx(ptim, tid, 0);
}

STDMETHODIMP CTextService::ActivateEx(ITfThreadMgr* ptim, TfClientId tid, DWORD /*dwFlags*/) {
    _pThreadMgr = ptim;
    _pThreadMgr->AddRef();
    _tid = tid;

    ITfSource* pSource = nullptr;
    if (SUCCEEDED(_pThreadMgr->QueryInterface(IID_ITfSource, reinterpret_cast<void**>(&pSource)))) {
        pSource->AdviseSink(IID_ITfThreadMgrEventSink,
                            static_cast<ITfThreadMgrEventSink*>(this), &_dwThreadMgrCookie);
        pSource->Release();
    }

    ITfKeystrokeMgr* pKeyMgr = nullptr;
    if (SUCCEEDED(_pThreadMgr->QueryInterface(IID_ITfKeystrokeMgr, reinterpret_cast<void**>(&pKeyMgr)))) {
        pKeyMgr->AdviseKeyEventSink(_tid, static_cast<ITfKeyEventSink*>(this), TRUE);
        pKeyMgr->Release();
    }

    _pipe = std::make_unique<romaji::PipeClient>(kPipeName);

    // Message-only window for off-keystroke AI polling (see StartAiConvert).
    WNDCLASSW wc = {};
    wc.lpfnWndProc = &CTextService::TimerWndProc;
    wc.hInstance = g_hInst;
    wc.lpszClassName = L"RomajiIMETimerWindow";
    RegisterClassW(&wc);  // harmless if already registered
    _hwndTimer = CreateWindowExW(0, L"RomajiIMETimerWindow", L"", 0, 0, 0, 0, 0,
                                 HWND_MESSAGE, nullptr, g_hInst, nullptr);
    if (_hwndTimer) {
        SetWindowLongPtrW(_hwndTimer, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(this));
    }
    return S_OK;
}

STDMETHODIMP CTextService::Deactivate() {
    StopAiTimer();
    if (_hwndTimer) {
        DestroyWindow(_hwndTimer);
        _hwndTimer = nullptr;
    }
    if (_pComposition) {
        _pComposition->Release();
        _pComposition = nullptr;
    }
    if (_pThreadMgr) {
        ITfKeystrokeMgr* pKeyMgr = nullptr;
        if (SUCCEEDED(_pThreadMgr->QueryInterface(IID_ITfKeystrokeMgr, reinterpret_cast<void**>(&pKeyMgr)))) {
            pKeyMgr->UnadviseKeyEventSink(_tid);
            pKeyMgr->Release();
        }
        if (_dwThreadMgrCookie != TF_INVALID_COOKIE) {
            ITfSource* pSource = nullptr;
            if (SUCCEEDED(_pThreadMgr->QueryInterface(IID_ITfSource, reinterpret_cast<void**>(&pSource)))) {
                pSource->UnadviseSink(_dwThreadMgrCookie);
                pSource->Release();
            }
            _dwThreadMgrCookie = TF_INVALID_COOKIE;
        }
        _pThreadMgr->Release();
        _pThreadMgr = nullptr;
    }
    if (_pipe && _hasSession) {
        _pipe->CloseSession(_sid);
    }
    _hasSession = false;
    _pipe.reset();
    return S_OK;
}

// ---------------------------------------------------------------------------
// ITfThreadMgrEventSink (unused for M1)
// ---------------------------------------------------------------------------
STDMETHODIMP CTextService::OnInitDocumentMgr(ITfDocumentMgr*) { return S_OK; }
STDMETHODIMP CTextService::OnUninitDocumentMgr(ITfDocumentMgr*) { return S_OK; }
STDMETHODIMP CTextService::OnSetFocus(ITfDocumentMgr*, ITfDocumentMgr*) { return S_OK; }
STDMETHODIMP CTextService::OnPushContext(ITfContext*) { return S_OK; }
STDMETHODIMP CTextService::OnPopContext(ITfContext*) { return S_OK; }

// ---------------------------------------------------------------------------
// ITfKeyEventSink
// ---------------------------------------------------------------------------

STDMETHODIMP CTextService::OnSetFocus(BOOL) { return S_OK; }

uint32_t CTextService::TranslateKey(WPARAM wParam, uint32_t* outMods) {
    uint32_t mods = 0;
    if (GetKeyState(VK_SHIFT) & 0x8000) mods |= 1;          // SHIFT
    if (GetKeyState(VK_CONTROL) & 0x8000) mods |= 1 << 2;   // CONTROL
    if (GetKeyState(VK_MENU) & 0x8000) mods |= 1 << 3;      // ALT
    *outMods = mods;

    switch (wParam) {
        case VK_BACK: return 0xFF08;
        case VK_RETURN: return 0xFF0D;
        case VK_ESCAPE: return 0xFF1B;
        case VK_SPACE: return 0x20;
        case VK_TAB: return 0xFF09;
        case VK_LEFT: return 0xFF51;
        case VK_UP: return 0xFF52;
        case VK_RIGHT: return 0xFF53;
        case VK_DOWN: return 0xFF54;
        case VK_OEM_MINUS: return '-';
        case VK_OEM_PERIOD: return '.';
        case VK_OEM_COMMA: return ',';
        default: break;
    }
    if (wParam >= 'A' && wParam <= 'Z') {
        return static_cast<uint32_t>('a' + (wParam - 'A'));  // engine is case-insensitive romaji
    }
    if (wParam >= '0' && wParam <= '9') {
        return static_cast<uint32_t>(wParam);
    }
    return 0;
}

STDMETHODIMP CTextService::OnTestKeyDown(ITfContext*, WPARAM wParam, LPARAM, BOOL* pfEaten) {
    *pfEaten = FALSE;
    uint32_t mods = 0;
    uint32_t sym = TranslateKey(wParam, &mods);
    if (sym == 0) return S_OK;
    if (mods & ((1 << 2) | (1 << 3))) return S_OK;  // Ctrl/Alt -> shortcut, not text

    const bool composing = (_pComposition != nullptr);
    if (sym >= 0x21 && sym <= 0x7E) {
        *pfEaten = TRUE;  // printable always starts/continues composition
    } else if (composing && (sym == 0xFF08 || sym == 0xFF0D || sym == 0xFF1B || sym == 0x20)) {
        *pfEaten = TRUE;  // command keys only while composing
    }
    return S_OK;
}

STDMETHODIMP CTextService::OnKeyDown(ITfContext* pic, WPARAM wParam, LPARAM, BOOL* pfEaten) {
    *pfEaten = FALSE;
    uint32_t mods = 0;
    uint32_t sym = TranslateKey(wParam, &mods);
    if (sym == 0) return S_OK;
    if (!EnsureSession()) return S_OK;  // server unavailable -> let the app handle the key

    // A new keystroke cancels any in-flight AI conversion (typing is never blocked).
    if (_aiActive) StopAiTimer();

    // Space triggers cloud-AI conversion (auto-convert-on-pause is a frontend
    // refinement; Enter commits as-is via ProcessKey). StartAiConvert returns
    // false when AI is unavailable or candidates already show, so we fall through.
    const bool ctrlAlt = (mods & ((1 << 2) | (1 << 3))) != 0;
    if (sym == 0x20 && !ctrlAlt && StartAiConvert(pic)) {
        *pfEaten = TRUE;
        return S_OK;
    }

    auto state = _pipe->ProcessKey(_sid, sym, mods);
    if (!state) return S_OK;  // soft failure -> fall back to the app
    if ((state->flags & romaji::kFlagConsumed) == 0) return S_OK;

    *pfEaten = TRUE;
    auto* pES = new CApplyStateEditSession(this, pic, *state);
    HRESULT hr = S_OK;
    pic->RequestEditSession(_tid, pES, TF_ES_READWRITE | TF_ES_SYNC, &hr);
    pES->Release();
    return S_OK;
}

STDMETHODIMP CTextService::OnTestKeyUp(ITfContext*, WPARAM, LPARAM, BOOL* pfEaten) {
    *pfEaten = FALSE;
    return S_OK;
}
STDMETHODIMP CTextService::OnKeyUp(ITfContext*, WPARAM, LPARAM, BOOL* pfEaten) {
    *pfEaten = FALSE;
    return S_OK;
}
STDMETHODIMP CTextService::OnPreservedKey(ITfContext*, REFGUID, BOOL* pfEaten) {
    *pfEaten = FALSE;
    return S_OK;
}

// ---------------------------------------------------------------------------
// Async cloud-AI conversion (polled off the keystroke path via WM_TIMER)
// ---------------------------------------------------------------------------

static constexpr UINT_PTR kAiTimerId = 1;

bool CTextService::StartAiConvert(ITfContext* pContext) {
    if (!_pipe || !_hwndTimer) return false;
    // TODO: pass surrounding document text as context (ITfContext range read).
    auto reqId = _pipe->BeginAiConvert(_sid, L"", L"");
    if (!reqId) return false;  // unavailable / not composing / candidates already shown
    _aiReqId = *reqId;
    if (_pAiContext) _pAiContext->Release();
    _pAiContext = pContext;
    _pAiContext->AddRef();
    _aiActive = true;
    _aiStartTick = GetTickCount();
    _aiLastCount = 0;
    _aiStablePolls = 0;
    SetTimer(_hwndTimer, kAiTimerId, 40, nullptr);  // poll the fast IPC every 40ms
    return true;
}

void CTextService::StopAiTimer() {
    if (_hwndTimer) KillTimer(_hwndTimer, kAiTimerId);
    _aiActive = false;
    if (_pAiContext) {
        _pAiContext->Release();
        _pAiContext = nullptr;
    }
}

void CTextService::PollAiOnce() {
    if (!_aiActive || !_pipe || !_pAiContext) return;

    auto outcome = _pipe->PollAiResult(_sid, _aiReqId);
    switch (outcome.kind) {
        case romaji::PipeClient::PollKind::Pending:
            break;  // not ready yet
        case romaji::PipeClient::PollKind::Ready: {
            // Render the current candidates inline (state.preedit = highlighted
            // candidate) by reusing the edit-session path.
            auto* pES = new CApplyStateEditSession(this, _pAiContext, outcome.state);
            HRESULT hr = S_OK;
            _pAiContext->RequestEditSession(_tid, pES, TF_ES_READWRITE | TF_ES_SYNC, &hr);
            pES->Release();
            // The dispatcher reports streaming and final identically, so stop once
            // the candidate list stops growing.
            if (outcome.state.candidates.size() == _aiLastCount) {
                if (++_aiStablePolls >= 3) StopAiTimer();
            } else {
                _aiLastCount = outcome.state.candidates.size();
                _aiStablePolls = 0;
            }
            break;
        }
        case romaji::PipeClient::PollKind::Error:
            StopAiTimer();  // leave the romaji for the user to edit/retry
            break;
    }
    if (_aiActive && (GetTickCount() - _aiStartTick) > 8000) {
        StopAiTimer();  // safety timeout
    }
}

LRESULT CALLBACK CTextService::TimerWndProc(HWND hwnd, UINT msg, WPARAM wParam, LPARAM lParam) {
    if (msg == WM_TIMER && wParam == kAiTimerId) {
        auto* self = reinterpret_cast<CTextService*>(GetWindowLongPtrW(hwnd, GWLP_USERDATA));
        if (self) self->PollAiOnce();
        return 0;
    }
    return DefWindowProcW(hwnd, msg, wParam, lParam);
}

// ---------------------------------------------------------------------------
// ITfCompositionSink
// ---------------------------------------------------------------------------
STDMETHODIMP CTextService::OnCompositionTerminated(TfEditCookie, ITfComposition* pComposition) {
    if (_pComposition == pComposition && _pComposition) {
        _pComposition->Release();
        _pComposition = nullptr;
    }
    return S_OK;
}

// ---------------------------------------------------------------------------
// Composition + state application (called inside an edit session)
// ---------------------------------------------------------------------------

bool CTextService::EnsureSession() {
    if (_hasSession) return true;
    if (!_pipe) return false;
    auto sid = _pipe->NewSession();
    if (!sid) return false;
    _sid = *sid;
    _hasSession = true;
    return true;
}

void CTextService::StartComposition(TfEditCookie ec, ITfContext* pContext) {
    if (_pComposition) return;

    ITfInsertAtSelection* pInsert = nullptr;
    if (FAILED(pContext->QueryInterface(IID_ITfInsertAtSelection, reinterpret_cast<void**>(&pInsert)))) {
        return;
    }
    ITfRange* pRange = nullptr;
    if (SUCCEEDED(pInsert->InsertTextAtSelection(ec, TF_IAS_QUERYONLY, nullptr, 0, &pRange)) && pRange) {
        ITfContextComposition* pCtxComp = nullptr;
        if (SUCCEEDED(pContext->QueryInterface(IID_ITfContextComposition,
                                               reinterpret_cast<void**>(&pCtxComp)))) {
            pCtxComp->StartComposition(ec, pRange, static_cast<ITfCompositionSink*>(this),
                                       &_pComposition);
            pCtxComp->Release();
        }
        pRange->Release();
    }
    pInsert->Release();
}

void CTextService::SetCompositionText(TfEditCookie ec, ITfContext* pContext, const std::wstring& text) {
    if (!_pComposition) return;
    ITfRange* pRange = nullptr;
    if (FAILED(_pComposition->GetRange(&pRange)) || !pRange) return;

    pRange->SetText(ec, 0, text.c_str(), static_cast<LONG>(text.size()));

    // Move the caret to the end of the composition.
    ITfRange* pEnd = nullptr;
    if (SUCCEEDED(pRange->Clone(&pEnd)) && pEnd) {
        pEnd->Collapse(ec, TF_ANCHOR_END);
        TF_SELECTION sel;
        sel.range = pEnd;
        sel.style.ase = TF_AE_END;
        sel.style.fInterimChar = FALSE;
        pContext->SetSelection(ec, 1, &sel);
        pEnd->Release();
    }
    pRange->Release();
}

void CTextService::EndComposition(TfEditCookie ec) {
    if (!_pComposition) return;
    _pComposition->EndComposition(ec);
    _pComposition->Release();
    _pComposition = nullptr;
}

void CTextService::ApplyStateInEditSession(TfEditCookie ec, ITfContext* pContext,
                                           const romaji::State& state) {
    if (!state.commit.empty()) {
        // Finalize the committed kana into the document.
        StartComposition(ec, pContext);
        SetCompositionText(ec, pContext, state.commit);
        EndComposition(ec);
    }
    if (!state.preedit.empty()) {
        StartComposition(ec, pContext);
        SetCompositionText(ec, pContext, state.preedit);
    } else if (state.commit.empty()) {
        // Nothing to show and nothing committed (e.g. Escape) -> clear.
        EndComposition(ec);
    }
}
