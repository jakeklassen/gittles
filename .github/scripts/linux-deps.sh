#!/usr/bin/env bash
# System libraries gpui links against on Linux. Package names are the ones
# proven to work on Ubuntu 24.04, which is what `ubuntu-latest` runs.
set -euxo pipefail

sudo apt-get update
sudo apt-get install -y --no-install-recommends \
  build-essential \
  clang \
  cmake \
  pkg-config \
  libasound2-dev \
  libfontconfig-dev \
  libfreetype-dev \
  libvulkan-dev \
  libwayland-dev \
  libx11-dev \
  libxcb1-dev \
  libxext-dev \
  libxkbcommon-dev \
  libxkbcommon-x11-dev
