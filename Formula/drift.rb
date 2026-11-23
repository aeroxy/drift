class Drift < Formula
  desc "Encrypted bidirectional file transfer over WebSocket with an embedded web UI"
  homepage "https://github.com/aeroxy/drift"
  version "0.5.3"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/aeroxy/drift/releases/download/#{version}/drift_macos_arm64.zip"
      sha256 "042a4e1c59f0b61f58791f78195b270f8ccff8cadcbe05746a1264f92af0d799"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/aeroxy/drift/releases/download/#{version}/drift_linux_x86_64.zip"
      sha256 "62e301a6def2bf4cc01bc3be71c6ec061576e49455bff3be8cb825175642bc62"
    end
  end

  def install
    bin.install "drift"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/drift --version")
  end
end
