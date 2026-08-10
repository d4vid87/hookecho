# Homebrew formula, for a personal tap (`brew tap d4vid87/hookecho`).
#
# Built from source rather than shipping a bottle: the release job's macOS artifact is ad-hoc
# signed and explicitly experimental, and `brew install` compiling from a tag is the honest
# version of that.
class Hookecho < Formula
  desc "Advanced NEXRAD weather radar viewer"
  homepage "https://github.com/d4vid87/hookecho"
  url "https://github.com/d4vid87/hookecho/archive/refs/tags/v0.8.0.tar.gz"
  # Filled in at tag time: `brew fetch --build-from-source hookecho` prints the checksum.
  sha256 "a77106b1869671f6c697cc41c62117518dc9b590eccb932917f258c6509845e6"
  license "MIT"
  head "https://github.com/d4vid87/hookecho.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/hookecho")
  end

  test do
    assert_match "hookecho", shell_output("#{bin}/hookecho --version")
    # The renderer needs a GPU, but the icon path is pure arithmetic and proves the binary runs.
    system bin/"hookecho", "--headless-icon", testpath/"icon.png", "64"
    assert_predicate testpath/"icon.png", :exist?
  end
end
