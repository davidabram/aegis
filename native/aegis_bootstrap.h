#ifndef AEGIS_NATIVE_AEGIS_BOOTSTRAP_H_
#define AEGIS_NATIVE_AEGIS_BOOTSTRAP_H_

#include <string_view>

namespace aegis {

inline constexpr std::string_view kBootstrapScheme = "https";
inline constexpr std::string_view kBootstrapDomain = "bootstrap.aegis";
inline constexpr std::string_view kBootstrapUrl = "https://bootstrap.aegis/";
inline constexpr std::string_view kLegacyCustomBootstrapUrl = "aegis://bootstrap/";
inline constexpr std::string_view kLegacyBootstrapUrl = "data:text/html,";

inline bool IsBootstrapUrl(std::string_view url) {
  return url == kBootstrapUrl || url == kLegacyCustomBootstrapUrl ||
         url == kLegacyBootstrapUrl;
}

}  // namespace aegis

#endif
