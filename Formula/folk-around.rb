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
      sha256 "690b0fff1e719bc47534d35e4ac62426f9138c599bca276171c640c830b29aa2"
    else
      url "https://github.com/undivisible/folk-around/releases/download/v0.3.0/folk-around-darwin-x86_64"
      sha256 "421c599ce1b57060b825eeb498a2e5196b8c5bb1f14a0b7db39934ecba51dca5"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/undivisible/folk-around/releases/download/v0.3.0/folk-around-linux-aarch64"
      sha256 "9284f392a4b01c02c94f40baceb08470e6345a631126b2d2d88a169971711fd7"
    else
      url "https://github.com/undivisible/folk-around/releases/download/v0.3.0/folk-around-linux-x86_64"
      sha256 "774ebce1406e1b95f15172832b020dd31b12948e6c3f19eee04c80dba8ee2da8"
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
