class Drift < Formula
  desc "Encrypted bidirectional file transfer over WebSocket with an embedded web UI"
  homepage "https://github.com/aeroxy/drift"
  version "0.2.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/aeroxy/drift/releases/download/#{version}/drift_macos_arm64.zip"
      sha256 "0019dfc4b32d63c1392aa264aed2253c1e0c2fb09216f8e2cc269bbfb8bb49b5"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/aeroxy/drift/releases/download/#{version}/drift_linux_x86_64.zip"
      sha256 "0019dfc4b32d63c1392aa264aed2253c1e0c2fb09216f8e2cc269bbfb8bb49b5"
    end
  end

  def install
    bin.install "drift"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/drift --version")
  end
end
