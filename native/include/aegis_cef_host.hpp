#ifndef AEGIS_CEF_HOST_HPP
#define AEGIS_CEF_HOST_HPP

#include <cstdint>
#include <memory>
#include <string>
#include <vector>

#include "aegis_protocol.hpp"
#include "aegis_host_abi.h"

namespace aegis {

class CefHost {
 public:
  virtual ~CefHost() = default;

  virtual std::vector<std::uint8_t> EnsureRuntime(const std::vector<std::uint8_t>& request) = 0;
  virtual std::vector<std::uint8_t> EvalJs(const std::vector<std::uint8_t>& request) = 0;
  virtual std::vector<std::uint8_t> SendBatch(const std::vector<std::uint8_t>& request) = 0;
  virtual std::vector<std::uint8_t> SnapshotDom(const std::vector<std::uint8_t>& request) = 0;
  virtual std::vector<std::uint8_t> InjectSession(const std::vector<std::uint8_t>& request) = 0;
  virtual std::vector<std::uint8_t> SnapshotSession(const std::vector<std::uint8_t>& request) = 0;
  virtual std::vector<std::uint8_t> DrainEvents(const std::vector<std::uint8_t>& request) = 0;
  virtual std::vector<std::uint8_t> Navigate(const std::vector<std::uint8_t>& request) = 0;
  virtual std::vector<std::uint8_t> SnapshotHostState(const std::vector<std::uint8_t>& request) = 0;
  virtual std::vector<std::uint8_t> Pump(const std::vector<std::uint8_t>& request) = 0;
  virtual void RequestCancel() = 0;
};

AegisHostFunctionTable ExportFunctionTable();

enum class EmbeddedHostOperation : std::uint16_t {
  EnsureRuntime = 1,
  EvalJs = 2,
  SendBatch = 3,
  SnapshotDom = 4,
  InjectSession = 5,
  SnapshotSession = 6,
  DrainEvents = 7,
  Navigate = 8,
  SnapshotHostState = 9,
};

bool RunEmbeddedHostOperation(const std::vector<std::uint8_t>& config,
                              EmbeddedHostOperation operation,
                              const std::vector<std::uint8_t>& request,
                              std::vector<std::uint8_t>* response,
                              std::string* error);

}  // namespace aegis

#endif
