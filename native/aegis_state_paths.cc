#include "aegis_state_paths.h"

#include <chrono>
#include <cstdlib>
#include <fstream>
#include <stdexcept>
#include <string>
#include <system_error>

#include <errno.h>
#include <signal.h>
#include <unistd.h>

namespace {

std::filesystem::path ExpandStateRoot() {
  if (const char* aegis_home = std::getenv("AEGIS_HOME");
      aegis_home != nullptr && *aegis_home != '\0') {
    return std::filesystem::path(aegis_home);
  }
  if (const char* home = std::getenv("HOME"); home != nullptr && *home != '\0') {
    return std::filesystem::path(home) / ".aegis";
  }
  throw std::runtime_error("HOME is not set");
}

std::filesystem::path ScopeInstancesDir(const std::string& scope) {
  return AegisRuntimeRoot() / scope / "instances";
}

bool ProcessAlive(pid_t pid) {
  if (pid <= 0) {
    return false;
  }
  if (::kill(pid, 0) == 0) {
    return true;
  }
  return errno == EPERM;
}

void EnsureDirectory(const std::filesystem::path& path) {
  std::error_code error;
  std::filesystem::create_directories(path, error);
  if (error) {
    throw std::runtime_error("failed to create directory: " + path.string());
  }
}

}  // namespace

std::filesystem::path AegisStateRoot() { return ExpandStateRoot(); }

std::filesystem::path AegisRuntimeRoot() { return AegisStateRoot() / "runtime"; }

void AegisCleanupStaleRuntimeSessions(const std::string& scope) {
  std::error_code error;
  const auto instances_dir = ScopeInstancesDir(scope);
  if (!std::filesystem::exists(instances_dir, error)) {
    return;
  }

  for (const auto& entry : std::filesystem::directory_iterator(instances_dir, error)) {
    if (error) {
      return;
    }
    if (!entry.is_directory()) {
      continue;
    }
    const auto name = entry.path().filename().string();
    const auto separator = name.find('-');
    if (separator == std::string::npos) {
      std::filesystem::remove_all(entry.path(), error);
      error.clear();
      continue;
    }
    pid_t pid = 0;
    try {
      pid = static_cast<pid_t>(std::stoll(name.substr(0, separator)));
    } catch (...) {
      std::filesystem::remove_all(entry.path(), error);
      error.clear();
      continue;
    }
    if (ProcessAlive(pid)) {
      continue;
    }
    std::filesystem::remove_all(entry.path(), error);
    error.clear();
  }
}

AegisRuntimeSessionPaths AegisCreateRuntimeSessionPaths(const std::string& scope) {
  AegisCleanupStaleRuntimeSessions(scope);

  const auto instances_dir = ScopeInstancesDir(scope);
  EnsureDirectory(instances_dir);

  const auto pid = static_cast<long long>(::getpid());
  const auto timestamp =
      std::chrono::duration_cast<std::chrono::microseconds>(
          std::chrono::system_clock::now().time_since_epoch())
          .count();
  const auto instance_dir =
      instances_dir / (std::to_string(pid) + "-" + std::to_string(timestamp));
  EnsureDirectory(instance_dir);

  std::ofstream marker(instance_dir / "session.json", std::ios::trunc);
  marker << "{\"pid\":" << pid << ",\"scope\":\"" << scope << "\"}";

  return {
      .instance_dir = instance_dir,
  };
}

void AegisRemoveRuntimeSession(const AegisRuntimeSessionPaths& paths) noexcept {
  if (paths.instance_dir.empty()) {
    return;
  }
  std::error_code error;
  std::filesystem::remove_all(paths.instance_dir, error);
}
