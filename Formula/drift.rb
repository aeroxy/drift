class Drift < Formula
  desc "Encrypted bidirectional file transfer over WebSocket with an embedded web UI"
  homepage "https://github.com/aeroxy/drift"
  version "0.3.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/aeroxy/drift/releases/download/#{version}/drift_macos_arm64.zip"
      sha256 "c917a3259df78b21c740fcb959e6386158934f126cbff677b1a041e1d3068f98"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/aeroxy/drift/releases/download/#{version}/drift_linux_x86_64.zip"
      sha256 "3331650c20079d44bb801c0d110400cb0e2f1985298bfd06715d4b1e980b1fbc"
    end
  end

  def install
    bin.install "drift"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/drift --version")
  end
end
