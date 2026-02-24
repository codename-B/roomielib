cmake_minimum_required(VERSION 3.10)
project(roomie_client CXX)

set(CMAKE_CXX_STANDARD 14)

add_library(roomie_client
  src/roomie_client.cpp
)
target_include_directories(roomie_client PUBLIC
  $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
  $<INSTALL_INTERFACE:include>
)

# Optional: static by default so Raylib/GameMaker can link easily
option(ROOMIE_BUILD_SHARED "Build shared library" OFF)
if(ROOMIE_BUILD_SHARED)
  set_target_properties(roomie_client PROPERTIES
    WINDOWS_EXPORT_ALL_SYMBOLS ON
    POSITION_INDEPENDENT_CODE ON
  )
endif()

# Example / test executable (optional)
option(ROOMIE_BUILD_EXAMPLE "Build example client" OFF)
if(ROOMIE_BUILD_EXAMPLE)
  add_executable(roomie_example example/main.cpp)
  target_link_libraries(roomie_example PRIVATE roomie_client)
  target_include_directories(roomie_example PRIVATE include)
endif()

# Raylib Pong example (optional)
option(ROOMIE_BUILD_PONG "Build Raylib Pong example" ON)
if(ROOMIE_BUILD_PONG)
  add_subdirectory(examples/raylib_pong)
endif()
