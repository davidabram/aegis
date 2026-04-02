#include "aegis_protocol.hpp"

#include <array>
#include <cstring>

namespace aegis {
namespace {

constexpr std::array<char, 4> kMagic = {'A', 'E', 'G', 'S'};
constexpr std::size_t kHeaderLen = 16;

std::uint16_t ReadU16(const std::uint8_t* ptr) {
  return static_cast<std::uint16_t>(ptr[0]) |
         (static_cast<std::uint16_t>(ptr[1]) << 8);
}

std::uint64_t ReadU64(const std::uint8_t* ptr) {
  std::uint64_t value = 0;
  for (int i = 0; i < 8; ++i) {
    value |= static_cast<std::uint64_t>(ptr[i]) << (8 * i);
  }
  return value;
}

void WriteU16(std::uint8_t* ptr, std::uint16_t value) {
  ptr[0] = static_cast<std::uint8_t>(value & 0xff);
  ptr[1] = static_cast<std::uint8_t>((value >> 8) & 0xff);
}

void WriteU64(std::uint8_t* ptr, std::uint64_t value) {
  for (int i = 0; i < 8; ++i) {
    ptr[i] = static_cast<std::uint8_t>((value >> (8 * i)) & 0xff);
  }
}

const char* MessageKindName(MessageKind kind) {
  switch (kind) {
    case MessageKind::InstallRuntime:
      return "InstallRuntime";
    case MessageKind::EvalJs:
      return "EvalJs";
    case MessageKind::SendBatch:
      return "SendBatch";
    case MessageKind::SnapshotDom:
      return "SnapshotDom";
    case MessageKind::InjectSession:
      return "InjectSession";
    case MessageKind::SnapshotSession:
      return "SnapshotSession";
    case MessageKind::DrainEvents:
      return "DrainEvents";
    case MessageKind::Navigate:
      return "Navigate";
    case MessageKind::SnapshotHostState:
      return "SnapshotHostState";
  }

  throw ProtocolError("unsupported message kind");
}

MessageKind ParseMessageKind(CefRefPtr<CefDictionaryValue> envelope) {
  if (!envelope->HasKey("kind")) {
    throw ProtocolError("protocol envelope is missing kind");
  }

  const auto type = envelope->GetType("kind");
  if (type == VTYPE_INT) {
    return static_cast<MessageKind>(envelope->GetInt("kind"));
  }
  if (type == VTYPE_STRING) {
    const auto kind = envelope->GetString("kind").ToString();
    if (kind == "InstallRuntime") {
      return MessageKind::InstallRuntime;
    }
    if (kind == "EvalJs") {
      return MessageKind::EvalJs;
    }
    if (kind == "SendBatch") {
      return MessageKind::SendBatch;
    }
    if (kind == "SnapshotDom") {
      return MessageKind::SnapshotDom;
    }
    if (kind == "InjectSession") {
      return MessageKind::InjectSession;
    }
    if (kind == "SnapshotSession") {
      return MessageKind::SnapshotSession;
    }
    if (kind == "DrainEvents") {
      return MessageKind::DrainEvents;
    }
    if (kind == "Navigate") {
      return MessageKind::Navigate;
    }
    if (kind == "SnapshotHostState") {
      return MessageKind::SnapshotHostState;
    }
  }

  throw ProtocolError("unsupported protocol message kind");
}

CefRefPtr<CefDictionaryValue> RequireDictionary(CefRefPtr<CefValue> value,
                                                const char* message) {
  if (!value.get() || value->GetType() != VTYPE_DICTIONARY) {
    std::string detail(message);
    detail += " (type=";
    detail += std::to_string(value.get() ? static_cast<int>(value->GetType()) : -1);
    detail += ")";
    throw ProtocolError(detail);
  }
  return value->GetDictionary()->Copy(false);
}

}  // namespace

CefRefPtr<CefValue> DecodeEnvelope(MessageKind expected_kind,
                                   const std::vector<std::uint8_t>& bytes) {
  if (bytes.size() < kHeaderLen) {
    throw ProtocolError("frame too short");
  }
  if (!std::equal(kMagic.begin(), kMagic.end(), bytes.begin())) {
    throw ProtocolError("bad magic");
  }

  const auto version = ReadU16(bytes.data() + 4);
  if (version != kProtocolVersion) {
    throw ProtocolError("unsupported protocol version");
  }

  const auto kind = ReadU16(bytes.data() + 6);
  if (kind != static_cast<std::uint16_t>(expected_kind)) {
    throw ProtocolError("unexpected message kind");
  }

  const auto length = static_cast<std::size_t>(ReadU64(bytes.data() + 8));
  if (bytes.size() != kHeaderLen + length) {
    throw ProtocolError("frame length mismatch");
  }

  auto envelope = CefParseJSON(bytes.data() + kHeaderLen, length, JSON_PARSER_RFC);
  auto dict = RequireDictionary(envelope, "protocol envelope is not a dictionary");
  if (!dict->HasKey("payload")) {
    throw ProtocolError("protocol envelope is missing fields");
  }
  if (ParseMessageKind(dict) != expected_kind) {
    throw ProtocolError("envelope kind mismatch");
  }

  auto payload = dict->GetValue("payload");
  if (!payload.get()) {
    throw ProtocolError("protocol payload is missing");
  }
  return payload->Copy();
}

std::vector<std::uint8_t> EncodeEnvelope(MessageKind kind,
                                         CefRefPtr<CefDictionaryValue> payload) {
  auto value = CefValue::Create();
  value->SetDictionary(payload);
  return EncodeEnvelope(kind, value);
}

std::vector<std::uint8_t> EncodeEnvelope(MessageKind kind, CefRefPtr<CefValue> payload) {
  auto envelope = CefDictionaryValue::Create();
  envelope->SetString("kind", MessageKindName(kind));
  envelope->SetValue("payload", payload);

  auto envelope_value = CefValue::Create();
  envelope_value->SetDictionary(envelope);
  const auto json = CefWriteJSON(envelope_value, JSON_WRITER_DEFAULT).ToString();
  std::vector<std::uint8_t> bytes(kHeaderLen + json.size());
  std::memcpy(bytes.data(), kMagic.data(), kMagic.size());
  WriteU16(bytes.data() + 4, kProtocolVersion);
  WriteU16(bytes.data() + 6, static_cast<std::uint16_t>(kind));
  WriteU64(bytes.data() + 8, json.size());
  std::memcpy(bytes.data() + kHeaderLen, json.data(), json.size());
  return bytes;
}

}  // namespace aegis
