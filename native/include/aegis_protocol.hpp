#ifndef AEGIS_PROTOCOL_HPP
#define AEGIS_PROTOCOL_HPP

#include <cstdint>
#include <string>
#include <vector>

#include "cef_parser.h"

namespace aegis {

enum class MessageKind : std::uint16_t {
  EnsureRuntime = 1,
  EvalJs = 2,
  SendBatch = 3,
  SnapshotDom = 4,
  InjectSession = 5,
  SnapshotSession = 6,
  DrainEvents = 7,
  Navigate = 8,
  SnapshotHostState = 9,
  ActivateBrowser = 10,
};

class ProtocolError : public std::runtime_error {
 public:
  using std::runtime_error::runtime_error;
};

constexpr std::uint16_t kProtocolVersion = 1;

CefRefPtr<CefValue> DecodeEnvelope(MessageKind expected_kind,
                                   const std::vector<std::uint8_t>& bytes);

std::vector<std::uint8_t> EncodeEnvelope(MessageKind kind,
                                         CefRefPtr<CefValue> payload);

std::vector<std::uint8_t> EncodeEnvelope(MessageKind kind,
                                         CefRefPtr<CefDictionaryValue> payload);

}  // namespace aegis

#endif
