#ifndef AEGIS_NATIVE_AEGIS_MESSAGES_H_
#define AEGIS_NATIVE_AEGIS_MESSAGES_H_

namespace aegis {

inline constexpr char kAegisRequestMessage[] = "Aegis.Request";
inline constexpr char kAegisResponseMessage[] = "Aegis.Response";
inline constexpr char kAegisLifecycleMessage[] = "Aegis.Lifecycle";

inline constexpr char kLifecycleContextReady[] = "context_ready";

inline constexpr char kOpEnsureRuntime[] = "ensure_runtime";
inline constexpr char kOpEvalJs[] = "eval_js";
inline constexpr char kOpSendBatch[] = "send_batch";
inline constexpr char kOpSnapshotDom[] = "snapshot_dom";
inline constexpr char kOpInjectStorage[] = "inject_storage";
inline constexpr char kOpSnapshotStorage[] = "snapshot_storage";
inline constexpr char kOpDrainEvents[] = "drain_events";
inline constexpr char kOpNavigate[] = "navigate";

}  // namespace aegis

#endif
