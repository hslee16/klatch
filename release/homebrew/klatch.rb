class Klatch < Formula
  desc "Substrate for agent rooms: claims, asks, a board and a durable encrypted log"
  homepage "https://github.com/hslee16/klatch"
  version "0.1.0-dev.2"
  license "MIT"

  # An early pre-release: no outside security review, not for sensitive data.
  # Each archive holds the binary, LICENSE and THIRD-PARTY-LICENSES.txt, and is built by the release workflow of
  # hslee16/klatch (verify with `gh attestation verify`; see SECURITY.md in that repository).
  on_macos do
    on_arm do
      url "https://github.com/hslee16/klatch/releases/download/v0.1.0.dev2/klatch-0.1.0-dev.2-aarch64-apple-darwin.tar.gz"
      sha256 "b0fc61917d5fe074c10950b922824368a758a1d3ad2a791d554ba1c1d764d8a9"
    end
    on_intel do
      url "https://github.com/hslee16/klatch/releases/download/v0.1.0.dev2/klatch-0.1.0-dev.2-x86_64-apple-darwin.tar.gz"
      sha256 "26482262ea32854057d8d2e217accdb1981ee1624705a2c37a624c342fc605c0"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/hslee16/klatch/releases/download/v0.1.0.dev2/klatch-0.1.0-dev.2-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "4f37c00b6b484244f62ccd431cea3ce047081b7b5ad6172b9a195200a5dd9bd8"
    end
    on_intel do
      url "https://github.com/hslee16/klatch/releases/download/v0.1.0.dev2/klatch-0.1.0-dev.2-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "551946c2f28a85f452e0cbf2c84a0ca5952f531f302527b73d44c5574f88c087"
    end
  end

  def install
    bin.install "klatch"
    doc.install "LICENSE", "THIRD-PARTY-LICENSES.txt"
  end

  test do
    assert_match "klatch #{version}", shell_output("#{bin}/klatch --version")
    # a whole round trip in a throwaway store: the key goes in a directory under the test home
    ENV["KLATCH_HOME"] = testpath/"home"
    ENV["KLATCH_KEY_DIR"] = testpath/"keys"
    ENV["KLATCH_NAME"] = "brew-test"
    system bin/"klatch", "join", "lobby"
    system bin/"klatch", "post", "lobby", "hello from brew test"
    # a member never sees its own posts, so read as a second member
    ENV["KLATCH_NAME"] = "brew-reader"
    system bin/"klatch", "join", "lobby"
    assert_match "hello from brew test", shell_output("#{bin}/klatch read lobby --peek")
  end
end
