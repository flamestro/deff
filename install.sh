#!/usr/bin/env bash
set -euo pipefail

REPO_URL="https://github.com/flamestro/deff.git"
PROJECT_NAME="deff"

if ! command -v cargo >/dev/null 2>&1; then
  printf 'error: cargo is required but was not found in PATH.\n' >&2
  printf 'Install Rust: https://rust-lang.org/tools/install/\n' >&2
  exit 1
fi

if ! command -v git >/dev/null 2>&1; then
  printf 'error: git is required but was not found in PATH.\n' >&2
  exit 1
fi

tmp_dir="$(mktemp -d)"
repo_dir="${tmp_dir}/${PROJECT_NAME}"

cleanup() {
  rm -rf "${tmp_dir}"
}

trap cleanup EXIT

echo "Cloning ${REPO_URL} into a temporary directory..."
git clone --depth 1 "${REPO_URL}" "${repo_dir}"

echo "Installing ${PROJECT_NAME} with cargo..."
cargo install --path "${repo_dir}" --locked

echo "${PROJECT_NAME} installed successfully."
