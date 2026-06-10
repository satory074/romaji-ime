# Build the RomajiIME TSF text-service DLL (Windows only).
#
# Prerequisites on the build machine:
#   - Visual Studio 2022 (MSVC) + Windows SDK
#   - The ime-server.exe built separately: `cargo build --release -p ime-server`
#
# Configure & build both architectures (a TSF DLL must ship 32- and 64-bit):
#   cmake -B build-x64 -A x64   && cmake --build build-x64 --config Release
#   cmake -B build-x86 -A Win32 && cmake --build build-x86 --config Release
#
# Then register (per arch, elevated): regsvr32 RomajiIME.dll
cmake_minimum_required(VERSION 3.20)
project(RomajiIME LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

add_library(RomajiIME SHARED
    src/Globals.cpp
    src/ipc.cpp
    src/TextService.cpp
    src/dllmain.cpp
    src/RomajiIME.def
)

target_compile_definitions(RomajiIME PRIVATE UNICODE _UNICODE WIN32_LEAN_AND_MEAN)

# TSF is pure COM: the interface IIDs / CLSIDs (IID_ITf*, CLSID_TF_*) are in
# uuid.lib; the rest is ole32 (COM) + advapi32 (registry) + user32 (window/timer).
# (There is no msctf.lib.)
target_link_libraries(RomajiIME PRIVATE
    ole32
    oleaut32
    advapi32
    user32
    uuid
)

set_target_properties(RomajiIME PROPERTIES OUTPUT_NAME "RomajiIME")
