#!/usr/bin/env bash
set -euo pipefail

if [[ -n "${BASH_VERSION:-}" ]]; then
  shopt -s inherit_errexit 2>/dev/null || true
fi

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
RELEASE_BIN="$REPO_ROOT/target/aarch64-apple-darwin/release/aegis"
INSTALLED_APP="$HOME/Applications/Aegis.app"
INSTALLED_CLI="$INSTALLED_APP/Contents/MacOS/aegis_cli"

if [[ -t 1 ]]; then
  C_RESET=$'\033[0m'
  C_BOLD=$'\033[1m'
  C_BLUE=$'\033[34m'
  C_GREEN=$'\033[32m'
  C_YELLOW=$'\033[33m'
  C_RED=$'\033[31m'
else
  C_RESET=""
  C_BOLD=""
  C_BLUE=""
  C_GREEN=""
  C_YELLOW=""
  C_RED=""
fi

CURRENT_STEP=""

log_section() {
  printf '\n%s%s%s\n' "$C_BOLD$C_BLUE" "$1" "$C_RESET"
}

log_info() {
  printf '%s•%s %s\n' "$C_BLUE" "$C_RESET" "$1"
}

log_success() {
  printf '%s✓%s %s\n' "$C_GREEN" "$C_RESET" "$1"
}

log_warn() {
  printf '%s!%s %s\n' "$C_YELLOW" "$C_RESET" "$1"
}

log_error() {
  printf '%s✗%s %s\n' "$C_RED" "$C_RESET" "$1" >&2
}

fail() {
  log_error "$1"
  exit 1
}

on_error() {
  local exit_code=$?
  if [[ -n "$CURRENT_STEP" ]]; then
    log_error "Install failed while: $CURRENT_STEP"
  else
    log_error "Install failed."
  fi
  log_error "If you need more detail, rerun this script and watch the step immediately above the failure."
  exit "$exit_code"
}

trap on_error ERR

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail "Missing required command: $1"
}

run_step() {
  CURRENT_STEP="$1"
  log_info "$CURRENT_STEP"
  shift
  "$@"
}

run_quiet_step() {
  CURRENT_STEP="$1"
  log_info "$CURRENT_STEP"
  shift
  local temp_log
  temp_log="$(mktemp)"
  if "$@" >"$temp_log" 2>&1; then
    rm -f "$temp_log"
    return 0
  fi
  cat "$temp_log" >&2
  rm -f "$temp_log"
  return 1
}

print_summary() {
  log_section "Install Complete"
  log_success "Installed app bundle: $INSTALLED_APP"
  log_success "Installed CLI: $INSTALLED_CLI"
  log_success "Canonical state root: ${AEGIS_HOME:-$HOME/.aegis}"
  printf '\n'
  log_info "Open the browser with: aegis"
  log_info "Run the automation server with: aegis serve --addr 127.0.0.1:7878"
  log_info "Inspect config with: aegis config get agent"
  log_info "Inspect secrets with: aegis config secrets-get --profile default"
}

log_section "Aegis Installer"
log_info "Repo: $REPO_ROOT"
log_info "This will build the release binary, install the local app bundle, and bootstrap ~/.aegis."

CURRENT_STEP="checking local prerequisites"
require_command cargo
require_command cmake
require_command xcodebuild
require_command codesign
require_command python3

if [[ ! -d "$REPO_ROOT/third_party/cef" ]]; then
  fail "CEF SDK is missing under $REPO_ROOT/third_party/cef. Install the local CEF bundle before running install."
fi

cd "$REPO_ROOT"

log_section "Build"
run_step "building release Rust binary" cargo build --release

if [[ ! -x "$RELEASE_BIN" ]]; then
  fail "Expected release binary at $RELEASE_BIN after build, but it was not found."
fi

log_section "Install"
run_quiet_step "installing the local Aegis app bundle" "$RELEASE_BIN" native install

if [[ ! -x "$INSTALLED_CLI" ]]; then
  fail "Expected installed CLI at $INSTALLED_CLI, but it was not found."
fi

log_section "Bootstrap State"
run_quiet_step "bootstrapping canonical Aegis config" "$INSTALLED_CLI" config get agent
run_quiet_step "bootstrapping canonical Aegis runtime config" "$INSTALLED_CLI" config get runtime
run_quiet_step "bootstrapping canonical Aegis secrets store" "$INSTALLED_CLI" config secrets-get --profile default

log_section "Verify"
run_quiet_step "verifying installed CLI surface" "$INSTALLED_CLI" config --help
run_quiet_step "verifying installed native paths" "$INSTALLED_CLI" native paths

if [[ ! -d "${AEGIS_HOME:-$HOME/.aegis}" ]]; then
  fail "Expected canonical Aegis state directory at ${AEGIS_HOME:-$HOME/.aegis}, but it was not created."
fi

print_summary
