class Klatch < Formula
  desc "Substrate for agent rooms: claims, asks, a board and a durable encrypted log"
  homepage "https://github.com/hslee16/klatch"
  version "0.2.0-alpha.1"
  license "MIT"

  # An early pre-release: no outside security review, not for sensitive data.
  # Each archive holds the binary, LICENSE and THIRD-PARTY-LICENSES.txt, and is built by the release workflow of
  # hslee16/klatch (verify with `gh attestation verify`; see SECURITY.md in that repository).
  on_macos do
    on_arm do
      url "https://github.com/hslee16/klatch/releases/download/v0.2.0a1/klatch-0.2.0-alpha.1-aarch64-apple-darwin.tar.gz"
      sha256 "55970fed84011d6e9bb75e4db2a1f1dc125061750270ad1c753d6e15d7ff5328"
    end
    on_intel do
      url "https://github.com/hslee16/klatch/releases/download/v0.2.0a1/klatch-0.2.0-alpha.1-x86_64-apple-darwin.tar.gz"
      sha256 "bb1524ff1f38c270b11c94bd300f2e39e7fd9ec4646223c63fb9a8655b4aa0f3"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/hslee16/klatch/releases/download/v0.2.0a1/klatch-0.2.0-alpha.1-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "08cf4461a9aed7cbf8df7c37286a6b040aadd8c0c848a23662aeef55209b4a73"
    end
    on_intel do
      url "https://github.com/hslee16/klatch/releases/download/v0.2.0a1/klatch-0.2.0-alpha.1-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "f04d6163961610e565626cc940112b34fb7221e8b46bad9fb6a40117dc932b39"
    end
  end

  def install
    bin.install "klatch", "klatch-server"
    doc.install "LICENSE", "THIRD-PARTY-LICENSES.txt"
  end

  test do
    assert_match "klatch #{version}", shell_output("#{bin}/klatch --version")
    assert_match "usage: klatch-server", shell_output("#{bin}/klatch-server --help")
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
