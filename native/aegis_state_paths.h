#ifndef AEGIS_NATIVE_AEGIS_STATE_PATHS_H_
#define AEGIS_NATIVE_AEGIS_STATE_PATHS_H_

#include <filesystem>
#include <string>

struct AegisRuntimeSessionPaths {
  std::filesystem::path instance_dir;
  std::filesystem::path root_cache_path;
};

std::filesystem::path AegisStateRoot();
std::filesystem::path AegisRuntimeRoot();
AegisRuntimeSessionPaths AegisCreateRuntimeSessionPaths(const std::string& scope);
void AegisCleanupStaleRuntimeSessions(const std::string& scope);
void AegisRemoveRuntimeSession(const AegisRuntimeSessionPaths& paths) noexcept;

#endif  // AEGIS_NATIVE_AEGIS_STATE_PATHS_H_
