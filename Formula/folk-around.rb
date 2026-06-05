# typed: false
# frozen_string_literal: true

class FolkAround < Formula
  desc "Rust MCP agent for computer control"
  homepage "https://folkaround.undivisible.dev"
  license "MPL-2.0"
  version "0.3.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/undivisible/folk-around/releases/download/v0.3.0/folk-around-darwin-aarch64"
      sha256 "1915ae43fd8f67856e39b57f6f78006e14a08b3d8068a41df9f0f62ab4fb7171"
    else
      url "https://github.com/undivisible/folk-around/releases/download/v0.3.0/folk-around-darwin-x86_64"
      sha256 "c73d124d64d7d9f4129650476efd44ad16bca156c61c1bbe5bfe55ac60bdeb8b"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/undivisible/folk-around/releases/download/v0.3.0/folk-around-linux-aarch64"
      sha256 "e8d28076c11d5800d8fc0151f11837318caf168f9c4116224ce39c499de5fd6d"
    else
      url "https://github.com/undivisible/folk-around/releases/download/v0.3.0/folk-around-linux-x86_64"
      sha256 "b48f98d6d0e4f498eb822743a7e409add47f1e4b6d22b056538485ddb0551e8d"
    end
  end

  def install
    binary = Dir["folk-around-*"].first || "folk-around"
    bin.install binary => "folk-around"
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
