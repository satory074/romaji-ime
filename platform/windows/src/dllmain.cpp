// DLL entry points, COM class factory, and TSF registration for RomajiIME.
#include "Globals.h"
#include "TextService.h"

#include <new>
#include <string>

// ---------------------------------------------------------------------------
// Class factory
// ---------------------------------------------------------------------------
namespace {
class CClassFactory final : public IClassFactory {
public:
    CClassFactory() : _cRef(1) {}

    STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override {
        if (!ppv) return E_INVALIDARG;
        if (IsEqualIID(riid, IID_IUnknown) || IsEqualIID(riid, IID_IClassFactory)) {
            *ppv = static_cast<IClassFactory*>(this);
            AddRef();
            return S_OK;
        }
        *ppv = nullptr;
        return E_NOINTERFACE;
    }
    STDMETHODIMP_(ULONG) AddRef() override { return InterlockedIncrement(&_cRef); }
    STDMETHODIMP_(ULONG) Release() override {
        LONG c = InterlockedDecrement(&_cRef);
        if (c == 0) delete this;
        return c;
    }
    STDMETHODIMP CreateInstance(IUnknown* pUnkOuter, REFIID riid, void** ppv) override {
        if (pUnkOuter) return CLASS_E_NOAGGREGATION;
        auto* pTS = new (std::nothrow) CTextService();
        if (!pTS) return E_OUTOFMEMORY;
        HRESULT hr = pTS->QueryInterface(riid, ppv);
        pTS->Release();
        return hr;
    }
    STDMETHODIMP LockServer(BOOL fLock) override {
        if (fLock) DllAddRef();
        else DllRelease();
        return S_OK;
    }

private:
    LONG _cRef;
};
}  // namespace

// ---------------------------------------------------------------------------
// DLL exports
// ---------------------------------------------------------------------------

STDAPI DllGetClassObject(REFCLSID rclsid, REFIID riid, void** ppv) {
    if (IsEqualCLSID(rclsid, c_clsidRomajiIME)) {
        auto* pFactory = new (std::nothrow) CClassFactory();
        if (!pFactory) return E_OUTOFMEMORY;
        HRESULT hr = pFactory->QueryInterface(riid, ppv);
        pFactory->Release();
        return hr;
    }
    return CLASS_E_CLASSNOTAVAILABLE;
}

STDAPI DllCanUnloadNow() { return (g_cDllRef == 0) ? S_OK : S_FALSE; }

// ---- registration helpers -------------------------------------------------
namespace {

void GuidToString(REFGUID guid, wchar_t out[40]) { StringFromGUID2(guid, out, 40); }

// HKCR\CLSID\{clsid} + InprocServer32 (ThreadingModel=Apartment).
HRESULT RegisterComServer() {
    wchar_t clsid[40];
    GuidToString(c_clsidRomajiIME, clsid);

    wchar_t module[MAX_PATH];
    if (GetModuleFileNameW(g_hInst, module, MAX_PATH) == 0) return E_FAIL;

    std::wstring key = std::wstring(L"CLSID\\") + clsid;
    HKEY hKey = nullptr;
    if (RegCreateKeyExW(HKEY_CLASSES_ROOT, key.c_str(), 0, nullptr, 0, KEY_WRITE, nullptr, &hKey,
                        nullptr) != ERROR_SUCCESS) {
        return E_FAIL;
    }
    RegSetValueExW(hKey, nullptr, 0, REG_SZ, reinterpret_cast<const BYTE*>(kDescription),
                   static_cast<DWORD>((wcslen(kDescription) + 1) * sizeof(wchar_t)));

    HKEY hInproc = nullptr;
    if (RegCreateKeyExW(hKey, L"InprocServer32", 0, nullptr, 0, KEY_WRITE, nullptr, &hInproc,
                        nullptr) == ERROR_SUCCESS) {
        RegSetValueExW(hInproc, nullptr, 0, REG_SZ, reinterpret_cast<const BYTE*>(module),
                       static_cast<DWORD>((wcslen(module) + 1) * sizeof(wchar_t)));
        const wchar_t kApartment[] = L"Apartment";
        RegSetValueExW(hInproc, L"ThreadingModel", 0, REG_SZ,
                       reinterpret_cast<const BYTE*>(kApartment), sizeof(kApartment));
        RegCloseKey(hInproc);
    }
    RegCloseKey(hKey);
    return S_OK;
}

void UnregisterComServer() {
    wchar_t clsid[40];
    GuidToString(c_clsidRomajiIME, clsid);
    std::wstring key = std::wstring(L"CLSID\\") + clsid;
    RegDeleteTreeW(HKEY_CLASSES_ROOT, key.c_str());
}

// TSF profile + categories.
HRESULT RegisterProfiles() {
    ITfInputProcessorProfiles* pProfiles = nullptr;
    HRESULT hr = CoCreateInstance(CLSID_TF_InputProcessorProfiles, nullptr, CLSCTX_INPROC_SERVER,
                                  IID_ITfInputProcessorProfiles,
                                  reinterpret_cast<void**>(&pProfiles));
    if (FAILED(hr)) return hr;

    hr = pProfiles->Register(c_clsidRomajiIME);
    if (SUCCEEDED(hr)) {
        hr = pProfiles->AddLanguageProfile(
            c_clsidRomajiIME, ROMAJI_LANGID, c_guidProfile, kDescription,
            static_cast<ULONG>(wcslen(kDescription)), nullptr, 0, 0);
    }
    pProfiles->Release();
    return hr;
}

void UnregisterProfiles() {
    ITfInputProcessorProfiles* pProfiles = nullptr;
    if (SUCCEEDED(CoCreateInstance(CLSID_TF_InputProcessorProfiles, nullptr, CLSCTX_INPROC_SERVER,
                                   IID_ITfInputProcessorProfiles,
                                   reinterpret_cast<void**>(&pProfiles)))) {
        pProfiles->Unregister(c_clsidRomajiIME);
        pProfiles->Release();
    }
}

HRESULT RegisterCategories() {
    ITfCategoryMgr* pCat = nullptr;
    HRESULT hr = CoCreateInstance(CLSID_TF_CategoryMgr, nullptr, CLSCTX_INPROC_SERVER,
                                  IID_ITfCategoryMgr, reinterpret_cast<void**>(&pCat));
    if (FAILED(hr)) return hr;

    // Keyboard processor, plus UI-less and Windows Store (immersive) support so
    // the IME works in modern/UWP apps (see plan's Windows requirements).
    const GUID* cats[] = {
        &GUID_TFCAT_TIP_KEYBOARD,
        &GUID_TFCAT_TIPCAP_UIELEMENTENABLED,
        &GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT,
        &GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT,
    };
    for (const GUID* cat : cats) {
        pCat->RegisterCategory(c_clsidRomajiIME, *cat, c_clsidRomajiIME);
    }
    pCat->Release();
    return S_OK;
}

void UnregisterCategories() {
    ITfCategoryMgr* pCat = nullptr;
    if (SUCCEEDED(CoCreateInstance(CLSID_TF_CategoryMgr, nullptr, CLSCTX_INPROC_SERVER,
                                   IID_ITfCategoryMgr, reinterpret_cast<void**>(&pCat)))) {
        const GUID* cats[] = {
            &GUID_TFCAT_TIP_KEYBOARD,
            &GUID_TFCAT_TIPCAP_UIELEMENTENABLED,
            &GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT,
            &GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT,
        };
        for (const GUID* cat : cats) {
            pCat->UnregisterCategory(c_clsidRomajiIME, *cat, c_clsidRomajiIME);
        }
        pCat->Release();
    }
}

}  // namespace

STDAPI DllRegisterServer() {
    if (FAILED(RegisterComServer())) return E_FAIL;
    HRESULT hr = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
    bool didInit = SUCCEEDED(hr);
    HRESULT result = E_FAIL;
    if (SUCCEEDED(RegisterProfiles()) && SUCCEEDED(RegisterCategories())) {
        result = S_OK;
    } else {
        UnregisterCategories();
        UnregisterProfiles();
        UnregisterComServer();
    }
    if (didInit) CoUninitialize();
    return result;
}

STDAPI DllUnregisterServer() {
    HRESULT hr = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
    bool didInit = SUCCEEDED(hr);
    UnregisterCategories();
    UnregisterProfiles();
    if (didInit) CoUninitialize();
    UnregisterComServer();
    return S_OK;
}

BOOL WINAPI DllMain(HINSTANCE hInstance, DWORD dwReason, LPVOID) {
    switch (dwReason) {
        case DLL_PROCESS_ATTACH:
            g_hInst = hInstance;
            DisableThreadLibraryCalls(hInstance);
            break;
        default:
            break;
    }
    return TRUE;
}
