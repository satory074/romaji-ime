// Shared globals, GUIDs, and constants for the RomajiIME TSF text service.
#pragma once

#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <windows.h>
#include <msctf.h>
#include <olectl.h>

extern HINSTANCE g_hInst;     // set in DllMain
extern LONG g_cDllRef;        // module reference count (objects + locks)

// CLSID of the text service (the TIP COM object).
// {7C4D9E2A-1B3F-4E6A-9C8D-2F5A6B7C8D9E}
extern const CLSID c_clsidRomajiIME;
// Language profile GUID.
// {3E8A1F4B-2C5D-4A7E-8B9F-1D2E3F4A5B6C}
extern const GUID c_guidProfile;

constexpr wchar_t kDescription[] = L"RomajiIME";
constexpr wchar_t kPipeName[] = L"\\\\.\\pipe\\romaji_ime";

// Japanese keyboard language id for the profile.
#define ROMAJI_LANGID MAKELANGID(LANG_JAPANESE, SUBLANG_DEFAULT)

inline void DllAddRef() { InterlockedIncrement(&g_cDllRef); }
inline void DllRelease() { InterlockedDecrement(&g_cDllRef); }
