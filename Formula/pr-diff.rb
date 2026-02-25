class PrDiff < Formula
  desc "Interactive side-by-side git diff viewer"
  homepage "https://github.com/your-org/pr-diff"
  url "https://github.com/your-org/pr-diff/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "REPLACE_WITH_SOURCE_TARBALL_SHA256"
  license "MIT"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: ".")
  end

  test do
    assert_match "side-by-side", shell_output("#{bin}/pr-diff --help")
  end
end
