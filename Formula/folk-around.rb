# typed: false
# frozen_string_literal: true

class FolkAround < Formula
  desc "Zig MCP agent for computer control"
  homepage "https://folkaround.undivisible.dev"
  license "MPL-2.0"
  version "0.2.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/undivisible/folk-around/releases/download/v0.2.0/folk-around-darwin-aarch64"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000" # placeholder
    else
      url "https://github.com/undivisible/folk-around/releases/download/v0.2.0/folk-around-darwin-x86_64"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000" # placeholder
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/undivisible/folk-around/releases/download/v0.2.0/folk-around-linux-aarch64"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000" # placeholder
    else
      url "https://github.com/undivisible/folk-around/releases/download/v0.2.0/folk-around-linux-x86_64"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000" # placeholder
    end
  end

  def install
    bin.install "folk-around"
    man1.install "scripts/folk-around.1" if File.exist?("scripts/folk-around.1")
  end

  def caveats
    <<~EOS
      folk-around is an MCP agent for computer control.

      Quick start:
        folk-around --mode full    # unrestricted (default)
        folk-around --mode sandbox # restricted mode
        folk-around --http 8080    # HTTP SSE transport

      For the menu bar companion:
        brew install --cask folk-around
    EOS
  end

  test do
    assert_match "folk-around", shell_output("#{bin}/folk-around --help")
  end
end
