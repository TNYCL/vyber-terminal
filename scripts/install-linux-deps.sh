#!/usr/bin/env bash
set -euo pipefail
sudo apt-get update
sudo apt-get install --no-install-recommends -y \
  build-essential pkg-config clang cmake libssl-dev libfontconfig1-dev \
  libfreetype6-dev libwayland-dev libxkbcommon-dev libxkbcommon-x11-dev \
  libxcb1-dev libvulkan-dev libegl1-mesa-dev libasound2-dev \
  mesa-vulkan-drivers fonts-dejavu-core fonts-noto-core fonts-noto-color-emoji \
  xdg-utils dbus-x11 xvfb zsh
