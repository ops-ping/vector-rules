cmake_minimum_required(VERSION 3.20)
project(vrules_embedding_wllama LANGUAGES C CXX)

if(NOT DEFINED WLLAMA_LLAMA_CPP_DIR)
    message(FATAL_ERROR "WLLAMA_LLAMA_CPP_DIR must point to wllama's pinned llama.cpp checkout")
endif()
if(NOT DEFINED VRULES_WIT_BINDINGS_DIR)
    message(FATAL_ERROR "VRULES_WIT_BINDINGS_DIR must contain generated C bindings")
endif()

set(BUILD_SHARED_LIBS OFF CACHE BOOL "" FORCE)
set(LLAMA_BUILD_COMMON OFF CACHE BOOL "" FORCE)
set(LLAMA_BUILD_TESTS OFF CACHE BOOL "" FORCE)
set(LLAMA_BUILD_TOOLS OFF CACHE BOOL "" FORCE)
set(LLAMA_BUILD_EXAMPLES OFF CACHE BOOL "" FORCE)
set(LLAMA_BUILD_SERVER OFF CACHE BOOL "" FORCE)
set(LLAMA_BUILD_APP OFF CACHE BOOL "" FORCE)
set(LLAMA_TOOLS_INSTALL OFF CACHE BOOL "" FORCE)
set(GGML_STATIC ON CACHE BOOL "" FORCE)
set(GGML_NATIVE OFF CACHE BOOL "" FORCE)
set(GGML_OPENMP OFF CACHE BOOL "" FORCE)
set(GGML_BLAS OFF CACHE BOOL "" FORCE)
set(GGML_LLAMAFILE OFF CACHE BOOL "" FORCE)
set(GGML_CPU_REPACK OFF CACHE BOOL "" FORCE)
set(GGML_CCACHE OFF CACHE BOOL "" FORCE)
set(GGML_ALL_WARNINGS OFF CACHE BOOL "" FORCE)
set(LLAMA_ALL_WARNINGS OFF CACHE BOOL "" FORCE)

add_compile_definitions(_WASI_EMULATED_SIGNAL _WASI_EMULATED_PROCESS_CLOCKS)
add_compile_options(-msimd128 -fwasm-exceptions -mllvm -wasm-use-legacy-eh=false)
add_link_options(-fwasm-exceptions)

# wasm-ld must be able to distinguish GGML objects that share a basename.
set(CMAKE_C_ARCHIVE_CREATE "<CMAKE_AR> qcT <TARGET> <LINK_FLAGS> <OBJECTS>")
set(CMAKE_CXX_ARCHIVE_CREATE "<CMAKE_AR> qcT <TARGET> <LINK_FLAGS> <OBJECTS>")

add_subdirectory("${WLLAMA_LLAMA_CPP_DIR}" llama.cpp)

set_property(
    SOURCE "${WLLAMA_LLAMA_CPP_DIR}/ggml/src/ggml-cpu/quants.c"
    DIRECTORY "${WLLAMA_LLAMA_CPP_DIR}/ggml/src"
    APPEND PROPERTY COMPILE_OPTIONS
    "-include${CMAKE_CURRENT_SOURCE_DIR}/src/ggml-wasi-fallback.h"
)

add_executable(vrules-embedding-wllama
    src/embedding.cpp
    "${VRULES_WIT_BINDINGS_DIR}/embedding_component.c"
    "${VRULES_WIT_BINDINGS_DIR}/embedding_component_component_type.o"
)
target_compile_features(vrules-embedding-wllama PRIVATE cxx_std_17)
target_compile_options(vrules-embedding-wllama PRIVATE -O3)
target_include_directories(vrules-embedding-wllama PRIVATE
    "${VRULES_WIT_BINDINGS_DIR}"
    "${WLLAMA_LLAMA_CPP_DIR}/include"
    "${WLLAMA_LLAMA_CPP_DIR}/vendor"
)
target_link_libraries(vrules-embedding-wllama PRIVATE
    llama
    dl
    unwind
    wasi-emulated-signal
    wasi-emulated-process-clocks
)
target_link_options(vrules-embedding-wllama PRIVATE
    -mexec-model=reactor
    -Wl,--no-entry
    -Wl,--strip-all
)
set_target_properties(vrules-embedding-wllama PROPERTIES SUFFIX ".wasm")
