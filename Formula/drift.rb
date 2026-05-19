class Drift < Formula
  desc "Encrypted bidirectional file transfer over WebSocket with an embedded web UI"
  homepage "https://github.com/aeroxy/drift"
  version "0.4.2"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/aeroxy/drift/releases/download/#{version}/drift_macos_arm64.zip"
      sha256 "def6517c0fc007f12eb21a43d3c1acf70d8bf6cf5774304dcd419813e46ca9a4"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/aeroxy/drift/releases/download/#{version}/drift_linux_x86_64.zip"
      sha256 "f78e4aff97a94aa1a5da8d9f6ead457dcd9ef0deecc8046e3bb8115b399368f5"
    end
  end

  def install
    bin.install "drift"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/drift --version")
  end
end
