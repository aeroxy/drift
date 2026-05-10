class Drift < Formula
  desc "Encrypted bidirectional file transfer over WebSocket with an embedded web UI"
  homepage "https://github.com/aeroxy/drift"
  version "0.4.1"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/aeroxy/drift/releases/download/#{version}/drift_macos_arm64.zip"
      sha256 "d1c5e52993e90bf0e1caf5b97939bcd482bee546701bac82f193d1b73e5065e4"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/aeroxy/drift/releases/download/#{version}/drift_linux_x86_64.zip"
      sha256 "9dee4249fb01c66e34c0cc67d33085370a22f76286acfa36135d41c35b68de95"
    end
  end

  def install
    bin.install "drift"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/drift --version")
  end
end
