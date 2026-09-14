cmake_minimum_required(VERSION 3.22.1)
project(hookecho_phonemize)

set(ESPEAK_ROOT "${CMAKE_CURRENT_LIST_DIR}/../../../../../target/piper/android/espeak-ng")
set(BUILD_SHARED_LIBS OFF CACHE BOOL "" FORCE)
set(USE_ASYNC OFF CACHE BOOL "" FORCE)
set(USE_MBROLA OFF CACHE BOOL "" FORCE)
set(USE_LIBSONIC OFF CACHE BOOL "" FORCE)
set(USE_LIBPCAUDIO OFF CACHE BOOL "" FORCE)
add_subdirectory("${ESPEAK_ROOT}" "${CMAKE_CURRENT_BINARY_DIR}/espeak")

add_library(hookecho_phonemize SHARED phonemize.cpp)
target_link_libraries(hookecho_phonemize PRIVATE espeak-ng log)
