#!/usr/bin/env bash
set -euo pipefail

echo "==> 安装 Ubuntu 构建依赖"
sudo apt update
sudo apt install -y \
  build-essential \
  pkg-config \
  libgtk-4-dev \
  libdbus-1-dev \
  libgtk4-layer-shell-dev \
  libssl-dev

if ! command -v cargo &>/dev/null; then
  echo "==> 未检测到 cargo，安装 Rust 工具链"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
fi

echo "==> 依赖安装完成。运行 cargo build --release 构建项目"
