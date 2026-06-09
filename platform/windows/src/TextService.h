// CTextService: the RomajiIME TSF Text Input Processor (TIP).
//
// Implements the modern TSF interfaces (ITfTextInputProcessorEx for UI-less
// support), receives key events, forwards them to ime-server over the named
// pipe, and reflects the returned preedit/commit into the document via TSF
// compositions. Candidate-window UI (ITfCandidateListUIElement) is M2.
//
// NOTE: this builds on Windows with the Windows SDK (msctf.h). It cannot be
// compiled on the macOS dev host; see platform/windows/README.md.
#pragma once

#include "Globals.h"
#include "ipc.h"

#include <memory>

class CTextService final : public ITfTextInputProcessorEx,
                           public ITfThreadMgrEventSink,
                           public ITfKeyEventSink,
                           public ITfCompositionSink {
public:
    CTextService();

    // IUnknown
    STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override;
    STDMETHODIMP_(ULONG) AddRef() override;
    STDMETHODIMP_(ULONG) Release() override;

    // ITfTextInputProcessor / Ex
    STDMETHODIMP Activate(ITfThreadMgr* ptim, TfClientId tid) override;
    STDMETHODIMP Deactivate() override;
    STDMETHODIMP ActivateEx(ITfThreadMgr* ptim, TfClientId tid, DWORD dwFlags) override;

    // ITfThreadMgrEventSink
    STDMETHODIMP OnInitDocumentMgr(ITfDocumentMgr*) override;
    STDMETHODIMP OnUninitDocumentMgr(ITfDocumentMgr*) override;
    STDMETHODIMP OnSetFocus(ITfDocumentMgr* pNew, ITfDocumentMgr* pPrev) override;
    STDMETHODIMP OnPushContext(ITfContext*) override;
    STDMETHODIMP OnPopContext(ITfContext*) override;

    // ITfKeyEventSink
    STDMETHODIMP OnSetFocus(BOOL fForeground) override;
    STDMETHODIMP OnTestKeyDown(ITfContext* pic, WPARAM wParam, LPARAM lParam, BOOL* pfEaten) override;
    STDMETHODIMP OnKeyDown(ITfContext* pic, WPARAM wParam, LPARAM lParam, BOOL* pfEaten) override;
    STDMETHODIMP OnTestKeyUp(ITfContext* pic, WPARAM wParam, LPARAM lParam, BOOL* pfEaten) override;
    STDMETHODIMP OnKeyUp(ITfContext* pic, WPARAM wParam, LPARAM lParam, BOOL* pfEaten) override;
    STDMETHODIMP OnPreservedKey(ITfContext* pic, REFGUID rguid, BOOL* pfEaten) override;

    // ITfCompositionSink
    STDMETHODIMP OnCompositionTerminated(TfEditCookie ecWrite, ITfComposition* pComposition) override;

    // Called from the edit session to apply an engine State to the document.
    void ApplyStateInEditSession(TfEditCookie ec, ITfContext* pContext, const romaji::State& state);

private:
    ~CTextService();

    bool EnsureSession();
    // Map a Win32 virtual key + modifiers to the engine's neutral keysym.
    // Returns 0 if this key should not be sent to the engine.
    uint32_t TranslateKey(WPARAM wParam, uint32_t* outMods);

    void StartComposition(TfEditCookie ec, ITfContext* pContext);
    void SetCompositionText(TfEditCookie ec, ITfContext* pContext, const std::wstring& text);
    void EndComposition(TfEditCookie ec);

    LONG _cRef;
    ITfThreadMgr* _pThreadMgr = nullptr;
    TfClientId _tid = TF_CLIENTID_NULL;
    DWORD _dwThreadMgrCookie = TF_INVALID_COOKIE;
    ITfComposition* _pComposition = nullptr;

    std::unique_ptr<romaji::PipeClient> _pipe;
    uint64_t _sid = 0;
    bool _hasSession = false;
};
