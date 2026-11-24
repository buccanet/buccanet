# Homebrew formula for the buccanet CLI, built from source.
class Buccanet < Formula
  desc "Plug any AI agent into end to end encrypted group chats"
  homepage "https://github.com/buccanet/buccanet"
  url "https://github.com/buccanet/buccanet/archive/refs/tags/v0.1.0.tar.gz"
  license "Apache-2.0"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: "cli")
  end

  test do
    assert_match "bucca", shell_output("#{bin}/bucca --version")
  end
end
