cmake_minimum_required(VERSION 3.14)
project(roomie_pong CXX)

set(CMAKE_CXX_STANDARD 11)

# Roomie client is built by parent (clients/cpp/CMakeLists.txt)
set(ROOMIE_CLIENT_DIR ${CMAKE_CURRENT_SOURCE_DIR}/../..)

# nlohmann/json (header-only)
include(FetchContent)
FetchContent_Declare(
  json
  GIT_REPOSITORY https://github.com/nlohmann/json.git
  GIT_TAG v3.11.3
  GIT_SHALLOW TRUE
)
FetchContent_GetProperties(json)
if(NOT json_POPULATED)
  FetchContent_Populate(json)
endif()

# Raylib: prefer local install at C:\raylib (or RAYLIB_ROOT) if present
if(NOT DEFINED RAYLIB_ROOT AND EXISTS "C:/raylib/include/raylib.h")
  set(RAYLIB_ROOT "C:/raylib" CACHE PATH "Raylib install root")
endif()
if(DEFINED RAYLIB_ROOT AND EXISTS "${RAYLIB_ROOT}/include/raylib.h")
  # Pre-built raylib
  if(EXISTS "${RAYLIB_ROOT}/lib/raylib.lib")
    set(RAYLIB_LIB "${RAYLIB_ROOT}/lib/raylib.lib")
  elseif(EXISTS "${RAYLIB_ROOT}/lib/x64/raylib.lib")
    set(RAYLIB_LIB "${RAYLIB_ROOT}/lib/x64/raylib.lib")
  else()
    set(RAYLIB_LIB "")
  endif()
  if(RAYLIB_LIB)
    add_library(raylib STATIC IMPORTED GLOBAL)
    set_target_properties(raylib PROPERTIES
      IMPORTED_LOCATION "${RAYLIB_LIB}"
      INTERFACE_INCLUDE_DIRECTORIES "${RAYLIB_ROOT}/include"
      INTERFACE_LINK_LIBRARIES "winmm;gdi32;opengl32"
    )
    message(STATUS "Using raylib from ${RAYLIB_ROOT}")
  endif()
endif()
if(NOT TARGET raylib)
  # Fallback: fetch and build raylib
  FetchContent_Declare(
    raylib
    GIT_REPOSITORY https://github.com/raysan5/raylib.git
    GIT_TAG 4.5.0
    GIT_SHALLOW TRUE
  )
  FetchContent_GetProperties(raylib)
  if(NOT raylib_POPULATED)
    FetchContent_Populate(raylib)
    set(BUILD_EXAMPLES OFF CACHE BOOL "" FORCE)
    add_subdirectory(${raylib_SOURCE_DIR} ${raylib_BINARY_DIR})
  endif()
endif()

add_executable(roomie_pong main.cpp)
target_include_directories(roomie_pong PRIVATE
  ${ROOMIE_CLIENT_DIR}/include
  ${json_SOURCE_DIR}/include
)
target_link_libraries(roomie_pong PRIVATE roomie_client raylib)

if(APPLE)
  target_link_libraries(roomie_pong PRIVATE "-framework IOKit" "-framework Cocoa" "-framework OpenGL")
endif()
