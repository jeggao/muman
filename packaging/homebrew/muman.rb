# The formula each release publishes to the Homebrew tap, filled in by
# packaging/render.sh.
class Muman < Formula
  desc "Keep a music library in step with a song list"
  homepage "https://github.com/jeggao/muman"
  version "@VERSION@"
  license "AGPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/jeggao/muman/releases/download/v@VERSION@/muman-v@VERSION@-aarch64-apple-darwin.tar.gz"
      sha256 "@SHA256:aarch64-apple-darwin@"
    end
    on_intel do
      url "https://github.com/jeggao/muman/releases/download/v@VERSION@/muman-v@VERSION@-x86_64-apple-darwin.tar.gz"
      sha256 "@SHA256:x86_64-apple-darwin@"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/jeggao/muman/releases/download/v@VERSION@/muman-v@VERSION@-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "@SHA256:aarch64-unknown-linux-gnu@"
    end
    on_intel do
      url "https://github.com/jeggao/muman/releases/download/v@VERSION@/muman-v@VERSION@-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "@SHA256:x86_64-unknown-linux-gnu@"
    end
  end

  depends_on "ffmpeg"
  depends_on "yt-dlp"

  def install
    bin.install "muman"
    generate_completions_from_executable(bin/"muman", "completions")
  end

  test do
    assert_match "muman #{version}", shell_output("#{bin}/muman --version")
  end
end
