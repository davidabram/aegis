cmake_minimum_required(VERSION 3.21)
project(aegis_native)
set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS ON)

find_package(Python3 REQUIRED COMPONENTS Interpreter)

if(NOT DEFINED AEGIS_TARGET_PLATFORM)
  if(APPLE)
    set(AEGIS_TARGET_PLATFORM "macos")
  elseif(UNIX)
    set(AEGIS_TARGET_PLATFORM "linux")
  else()
    message(FATAL_ERROR "Unsupported platform")
  endif()
endif()

if(NOT DEFINED AEGIS_TARGET_ARCH)
  if(CMAKE_SYSTEM_PROCESSOR MATCHES "^(aarch64|arm64)$")
    set(AEGIS_TARGET_ARCH "arm64")
  elseif(CMAKE_SYSTEM_PROCESSOR MATCHES "^(x86_64|AMD64)$")
    set(AEGIS_TARGET_ARCH "x86_64")
  else()
    message(FATAL_ERROR "Unsupported architecture ${CMAKE_SYSTEM_PROCESSOR}")
  endif()
endif()

if(NOT DEFINED PROJECT_ARCH)
  set(PROJECT_ARCH "${AEGIS_TARGET_ARCH}")
endif()

if(NOT DEFINED CEF_ROOT)
  set(AEGIS_CEF_VERSION "146.0.6+g68649e2+chromium-146.0.7680.154")
  if(AEGIS_TARGET_PLATFORM STREQUAL "macos" AND AEGIS_TARGET_ARCH STREQUAL "arm64")
    set(AEGIS_CEF_PLATFORM_SUFFIX "macosarm64")
  elseif(AEGIS_TARGET_PLATFORM STREQUAL "macos" AND AEGIS_TARGET_ARCH STREQUAL "x86_64")
    set(AEGIS_CEF_PLATFORM_SUFFIX "macosx64")
  elseif(AEGIS_TARGET_PLATFORM STREQUAL "linux" AND AEGIS_TARGET_ARCH STREQUAL "arm64")
    set(AEGIS_CEF_PLATFORM_SUFFIX "linuxarm64")
  elseif(AEGIS_TARGET_PLATFORM STREQUAL "linux" AND AEGIS_TARGET_ARCH STREQUAL "x86_64")
    set(AEGIS_CEF_PLATFORM_SUFFIX "linux64")
  else()
    message(FATAL_ERROR "Unsupported AEGIS target ${AEGIS_TARGET_PLATFORM}/${AEGIS_TARGET_ARCH}")
  endif()
  set(CEF_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../third_party/cef/cef_binary_${AEGIS_CEF_VERSION}_${AEGIS_CEF_PLATFORM_SUFFIX}")
endif()

set(CMAKE_MODULE_PATH ${CMAKE_MODULE_PATH} "${CEF_ROOT}/cmake")

find_package(CEF REQUIRED)
add_subdirectory(${CEF_LIBCEF_DLL_WRAPPER_PATH} libcef_dll_wrapper)

set(AEGIS_NATIVE_TARGET "aegis_native")
set(AEGIS_HOST_LIBRARY_TARGET "aegis_host")
set(AEGIS_SHARED_SUPPORT_TARGET "aegis_shared_support")
set(AEGIS_HELPER_TARGET "aegis_helper")
set(AEGIS_GENERATED_DIR "${CMAKE_CURRENT_BINARY_DIR}/generated")
set(AEGIS_RUNTIME_SCRIPT_JS "${CMAKE_CURRENT_SOURCE_DIR}/../assets/js/aegis_runtime.js")
set(AEGIS_RUNTIME_SCRIPT_HEADER "${AEGIS_GENERATED_DIR}/aegis_runtime_script.h")

add_custom_command(
  OUTPUT ${AEGIS_RUNTIME_SCRIPT_HEADER}
  COMMAND ${Python3_EXECUTABLE}
          ${CMAKE_CURRENT_SOURCE_DIR}/../scripts/generate_runtime_header.py
          ${AEGIS_RUNTIME_SCRIPT_JS}
          ${AEGIS_RUNTIME_SCRIPT_HEADER}
  DEPENDS
    ${AEGIS_RUNTIME_SCRIPT_JS}
    ${CMAKE_CURRENT_SOURCE_DIR}/../scripts/generate_runtime_header.py
  VERBATIM
)

add_custom_target(aegis_runtime_header DEPENDS ${AEGIS_RUNTIME_SCRIPT_HEADER})

SET_CEF_TARGET_OUT_DIR()

if(AEGIS_TARGET_PLATFORM STREQUAL "linux")
  ADD_LOGICAL_TARGET("libcef_lib" "${CEF_LIB_DEBUG}" "${CEF_LIB_RELEASE}")
endif()

if(AEGIS_TARGET_PLATFORM STREQUAL "macos")
  set(AEGIS_PLATFORM_SOURCES
    aegis_platform_mac.mm
  )
  set(AEGIS_HELPER_SOURCE process_helper_mac.cc)
  set(AEGIS_NATIVE_RESOURCES
    mac/Info.plist.in
    mac/helper-Info.plist.in
    mac/English.lproj/InfoPlist.strings
  )
elseif(AEGIS_TARGET_PLATFORM STREQUAL "linux")
  set(AEGIS_PLATFORM_SOURCES
    aegis_platform_linux.cc
  )
  set(AEGIS_HELPER_SOURCE process_helper_linux.cc)
  set(AEGIS_NATIVE_RESOURCES)
else()
  message(FATAL_ERROR "Unsupported AEGIS_TARGET_PLATFORM=${AEGIS_TARGET_PLATFORM}")
endif()

set(AEGIS_COMMON_SOURCES
  ${AEGIS_RUNTIME_SCRIPT_HEADER}
  aegis_app.h
  aegis_app.cc
  aegis_client.h
  aegis_client.cc
  aegis_state_paths.h
  aegis_state_paths.cc
  src/aegis_protocol.cpp
  src/aegis_cef_host.cpp
)

set(AEGIS_NATIVE_SOURCES
  aegis_native_main.cc
  ${AEGIS_COMMON_SOURCES}
  ${AEGIS_PLATFORM_SOURCES}
)

set(AEGIS_HELPER_SOURCES
  ${AEGIS_HELPER_SOURCE}
  aegis_app.cc
  aegis_client.cc
)

set(AEGIS_HOST_LIBRARY_SOURCES
  aegis_state_paths.cc
  src/aegis_protocol.cpp
  src/aegis_cef_host.cpp
  ${AEGIS_PLATFORM_SOURCES}
)

set_source_files_properties(
  src/aegis_protocol.cpp
  src/aegis_cef_host.cpp
  PROPERTIES COMPILE_FLAGS "-fexceptions"
)

function(aegis_target_includes target)
  target_include_directories(${target} PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}
    ${AEGIS_GENERATED_DIR}
    ${CMAKE_CURRENT_SOURCE_DIR}/include
    ${CEF_ROOT}
    ${CEF_ROOT}/include
  )
endfunction()

function(aegis_target_output_dirs target)
  if(AEGIS_TARGET_PLATFORM STREQUAL "macos")
    set_target_properties(${target} PROPERTIES
      RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/Release"
      LIBRARY_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/Release"
      RUNTIME_OUTPUT_DIRECTORY_DEBUG "${CMAKE_BINARY_DIR}/Debug"
      RUNTIME_OUTPUT_DIRECTORY_RELEASE "${CMAKE_BINARY_DIR}/Release"
      LIBRARY_OUTPUT_DIRECTORY_DEBUG "${CMAKE_BINARY_DIR}/Debug"
      LIBRARY_OUTPUT_DIRECTORY_RELEASE "${CMAKE_BINARY_DIR}/Release"
    )
  else()
    set_target_properties(${target} PROPERTIES
      RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/release"
      LIBRARY_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/release"
      RUNTIME_OUTPUT_DIRECTORY_DEBUG "${CMAKE_BINARY_DIR}/debug"
      RUNTIME_OUTPUT_DIRECTORY_RELEASE "${CMAKE_BINARY_DIR}/release"
      LIBRARY_OUTPUT_DIRECTORY_DEBUG "${CMAKE_BINARY_DIR}/debug"
      LIBRARY_OUTPUT_DIRECTORY_RELEASE "${CMAKE_BINARY_DIR}/release"
    )
  endif()
endfunction()

function(aegis_set_linux_rpath target runtime_paths)
  if(AEGIS_TARGET_PLATFORM STREQUAL "linux")
    set_target_properties(${target} PROPERTIES
      BUILD_RPATH "${runtime_paths}"
      INSTALL_RPATH "${runtime_paths}"
    )
  endif()
endfunction()

if(AEGIS_TARGET_PLATFORM STREQUAL "macos")
  add_executable(${AEGIS_NATIVE_TARGET} MACOSX_BUNDLE ${AEGIS_NATIVE_SOURCES} ${AEGIS_NATIVE_RESOURCES})
  SET_EXECUTABLE_TARGET_PROPERTIES(${AEGIS_NATIVE_TARGET})
  set_target_properties(${AEGIS_NATIVE_TARGET} PROPERTIES
    MACOSX_BUNDLE_INFO_PLIST ${CMAKE_CURRENT_SOURCE_DIR}/mac/Info.plist.in
    CLANG_ENABLE_OBJC_ARC YES
    XCODE_ATTRIBUTE_GCC_ENABLE_CPP_EXCEPTIONS YES
    XCODE_ATTRIBUTE_PRODUCT_BUNDLE_IDENTIFIER "com.ariacomputecompany.aegis"
  )
else()
  add_executable(${AEGIS_NATIVE_TARGET} ${AEGIS_NATIVE_SOURCES})
endif()
add_dependencies(${AEGIS_NATIVE_TARGET} aegis_runtime_header)
aegis_target_includes(${AEGIS_NATIVE_TARGET})
target_compile_options(${AEGIS_NATIVE_TARGET} PRIVATE -fexceptions)
target_compile_definitions(${AEGIS_NATIVE_TARGET} PRIVATE AEGIS_STANDALONE_APP=1)
if(AEGIS_TARGET_PLATFORM STREQUAL "linux")
  target_link_libraries(${AEGIS_NATIVE_TARGET} PRIVATE libcef_lib libcef_dll_wrapper ${CEF_STANDARD_LIBS})
else()
  target_link_libraries(${AEGIS_NATIVE_TARGET} PRIVATE libcef_dll_wrapper ${CEF_STANDARD_LIBS})
endif()
aegis_target_output_dirs(${AEGIS_NATIVE_TARGET})
aegis_set_linux_rpath(${AEGIS_NATIVE_TARGET} "$ORIGIN;$ORIGIN/../lib")
if(AEGIS_TARGET_PLATFORM STREQUAL "macos")
  target_link_libraries(${AEGIS_NATIVE_TARGET} PRIVATE "-framework QuartzCore")
endif()

add_library(${AEGIS_SHARED_SUPPORT_TARGET} STATIC
  ${AEGIS_RUNTIME_SCRIPT_HEADER}
  aegis_app.cc
  aegis_client.cc
  aegis_state_paths.cc
  ${AEGIS_PLATFORM_SOURCES}
)
add_dependencies(${AEGIS_SHARED_SUPPORT_TARGET} aegis_runtime_header)
aegis_target_includes(${AEGIS_SHARED_SUPPORT_TARGET})
set_target_properties(${AEGIS_SHARED_SUPPORT_TARGET} PROPERTIES POSITION_INDEPENDENT_CODE ON)
target_compile_features(${AEGIS_SHARED_SUPPORT_TARGET} PRIVATE cxx_std_20)
if(AEGIS_TARGET_PLATFORM STREQUAL "linux")
  target_link_libraries(${AEGIS_SHARED_SUPPORT_TARGET} PRIVATE
    libcef_lib
    libcef_dll_wrapper
    ${CEF_STANDARD_LIBS}
  )
else()
  target_link_libraries(${AEGIS_SHARED_SUPPORT_TARGET} PRIVATE
    libcef_dll_wrapper
    ${CEF_STANDARD_LIBS}
  )
endif()
if(AEGIS_TARGET_PLATFORM STREQUAL "macos")
  target_link_libraries(${AEGIS_SHARED_SUPPORT_TARGET} PRIVATE "-framework QuartzCore")
endif()

add_library(${AEGIS_HOST_LIBRARY_TARGET} SHARED ${AEGIS_HOST_LIBRARY_SOURCES})
aegis_target_includes(${AEGIS_HOST_LIBRARY_TARGET})
target_compile_features(${AEGIS_HOST_LIBRARY_TARGET} PRIVATE cxx_std_20)
if(AEGIS_TARGET_PLATFORM STREQUAL "linux")
  target_link_libraries(${AEGIS_HOST_LIBRARY_TARGET} PRIVATE
    ${AEGIS_SHARED_SUPPORT_TARGET}
    libcef_lib
    libcef_dll_wrapper
    ${CEF_STANDARD_LIBS}
  )
else()
  target_link_libraries(${AEGIS_HOST_LIBRARY_TARGET} PRIVATE
    ${AEGIS_SHARED_SUPPORT_TARGET}
    libcef_dll_wrapper
    ${CEF_STANDARD_LIBS}
  )
endif()
set_target_properties(${AEGIS_HOST_LIBRARY_TARGET} PROPERTIES OUTPUT_NAME "aegis_host")
aegis_target_output_dirs(${AEGIS_HOST_LIBRARY_TARGET})
aegis_set_linux_rpath(${AEGIS_HOST_LIBRARY_TARGET} "$ORIGIN")
if(AEGIS_TARGET_PLATFORM STREQUAL "macos")
  set_target_properties(${AEGIS_HOST_LIBRARY_TARGET} PROPERTIES
    XCODE_ATTRIBUTE_CODE_SIGNING_ALLOWED "NO"
    XCODE_ATTRIBUTE_CODE_SIGNING_REQUIRED "NO"
  )
endif()

if(AEGIS_TARGET_PLATFORM STREQUAL "macos")
  set(_helper_output_name "aegis_native Helper")
  set(_helper_info_plist "${CMAKE_CURRENT_BINARY_DIR}/helper-Info.plist")
  file(READ "${CMAKE_CURRENT_SOURCE_DIR}/mac/helper-Info.plist.in" _plist_contents)
  string(REPLACE "\${EXECUTABLE_NAME}" "${_helper_output_name}" _plist_contents ${_plist_contents})
  string(REPLACE "\${PRODUCT_NAME}" "${_helper_output_name}" _plist_contents ${_plist_contents})
  string(REPLACE "\${BUNDLE_ID_SUFFIX}" "" _plist_contents ${_plist_contents})
  file(WRITE ${_helper_info_plist} ${_plist_contents})

  add_executable(${AEGIS_HELPER_TARGET} MACOSX_BUNDLE ${AEGIS_HELPER_SOURCES})
  SET_EXECUTABLE_TARGET_PROPERTIES(${AEGIS_HELPER_TARGET})
else()
  add_executable(${AEGIS_HELPER_TARGET} ${AEGIS_HELPER_SOURCES})
endif()
add_dependencies(${AEGIS_HELPER_TARGET} aegis_runtime_header)
aegis_target_includes(${AEGIS_HELPER_TARGET})
if(AEGIS_TARGET_PLATFORM STREQUAL "linux")
  target_link_libraries(${AEGIS_HELPER_TARGET} PRIVATE libcef_lib libcef_dll_wrapper ${CEF_STANDARD_LIBS})
else()
  target_link_libraries(${AEGIS_HELPER_TARGET} PRIVATE libcef_dll_wrapper ${CEF_STANDARD_LIBS})
endif()
aegis_target_output_dirs(${AEGIS_HELPER_TARGET})
aegis_set_linux_rpath(${AEGIS_HELPER_TARGET} "$ORIGIN;$ORIGIN/../lib")
if(AEGIS_TARGET_PLATFORM STREQUAL "macos")
  set_target_properties(${AEGIS_HELPER_TARGET} PROPERTIES
    MACOSX_BUNDLE_INFO_PLIST ${_helper_info_plist}
    OUTPUT_NAME "${_helper_output_name}"
    CLANG_ENABLE_OBJC_ARC YES
    XCODE_ATTRIBUTE_PRODUCT_BUNDLE_IDENTIFIER "com.ariacomputecompany.aegis.helper"
  )
endif()

if(AEGIS_TARGET_PLATFORM STREQUAL "macos")
  set(CEF_APP "${CMAKE_BINARY_DIR}/Release/${AEGIS_NATIVE_TARGET}.app")
  add_dependencies(${AEGIS_NATIVE_TARGET} ${AEGIS_HOST_LIBRARY_TARGET} ${AEGIS_HELPER_TARGET})
  add_custom_command(
    TARGET ${AEGIS_NATIVE_TARGET}
    POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E copy_if_different
            "${CMAKE_BINARY_DIR}/Release/libaegis_host.dylib"
            "${CEF_APP}/Contents/Frameworks/libaegis_host.dylib"
    VERBATIM
  )
  COPY_MAC_FRAMEWORK("${AEGIS_NATIVE_TARGET}" "${CEF_BINARY_DIR}" "${CEF_APP}")

  add_custom_command(
    TARGET ${AEGIS_NATIVE_TARGET}
    POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E copy_directory
            "${CMAKE_BINARY_DIR}/Release/${_helper_output_name}.app"
            "${CEF_APP}/Contents/Frameworks/${_helper_output_name}.app"
    VERBATIM
  )

  set(PREFIXES "mac/")
  COPY_MAC_RESOURCES("${AEGIS_NATIVE_RESOURCES}" "${PREFIXES}" "${AEGIS_NATIVE_TARGET}" "${CMAKE_CURRENT_SOURCE_DIR}" "${CEF_APP}")
elseif(AEGIS_TARGET_PLATFORM STREQUAL "linux")
  add_dependencies(${AEGIS_NATIVE_TARGET} ${AEGIS_HOST_LIBRARY_TARGET} ${AEGIS_HELPER_TARGET})
  foreach(_runtime_file
      libcef.so
      libEGL.so
      libGLESv2.so
      libvk_swiftshader.so
      libvulkan.so.1
      chrome-sandbox
      icudtl.dat
      snapshot_blob.bin
      v8_context_snapshot.bin)
    if(EXISTS "${CEF_ROOT}/Release/${_runtime_file}")
      add_custom_command(
        TARGET ${AEGIS_NATIVE_TARGET}
        POST_BUILD
        COMMAND ${CMAKE_COMMAND} -E copy_if_different
                "${CEF_ROOT}/Release/${_runtime_file}"
                "${CMAKE_BINARY_DIR}/release/${_runtime_file}"
        VERBATIM
      )
    endif()
  endforeach()
  foreach(_resource_file
      icudtl.dat
      vk_swiftshader_icd.json
      chrome_100_percent.pak
      chrome_200_percent.pak
      resources.pak)
    if(EXISTS "${CEF_ROOT}/Resources/${_resource_file}")
      add_custom_command(
        TARGET ${AEGIS_NATIVE_TARGET}
        POST_BUILD
        COMMAND ${CMAKE_COMMAND} -E copy_if_different
                "${CEF_ROOT}/Resources/${_resource_file}"
                "${CMAKE_BINARY_DIR}/release/${_resource_file}"
        VERBATIM
      )
    endif()
  endforeach()
  foreach(_resource_dir locales swiftshader)
    if(EXISTS "${CEF_ROOT}/Resources/${_resource_dir}")
      add_custom_command(
        TARGET ${AEGIS_NATIVE_TARGET}
        POST_BUILD
        COMMAND ${CMAKE_COMMAND} -E copy_directory
                "${CEF_ROOT}/Resources/${_resource_dir}"
                "${CMAKE_BINARY_DIR}/release/${_resource_dir}"
        VERBATIM
      )
    endif()
  endforeach()
endif()
