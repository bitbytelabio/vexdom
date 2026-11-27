#!/bin/sh
# Prints the Homebrew cask for one Vexdom release (Casks/vexdom.rb in bitbytelabio/homebrew-tap).
# The release job runs it after publishing; the shas are the first field of each
# `Vexdom-<version>-<target>.dmg.sha256`:
#
#   scripts/render-cask.sh <version> <sha256-arm64> <sha256-x86_64>
set -eu

if [ "$#" -ne 3 ]; then
  echo "usage: $0 <version> <sha256-arm64> <sha256-x86_64>" >&2
  exit 2
fi

sed -e "s/@VERSION@/$1/" -e "s/@SHA_ARM@/$2/" -e "s/@SHA_INTEL@/$3/" <<'EOF'
cask "vexdom" do
  arch arm: "aarch64", intel: "x86_64"

  version "@VERSION@"
  sha256 arm:   "@SHA_ARM@",
         intel: "@SHA_INTEL@"

  url "https://github.com/bitbytelabio/vexdom/releases/download/v#{version}/Vexdom-#{version}-#{arch}-apple-darwin.dmg"
  name "Vexdom"
  desc "Drive your real browser from AI agents"
  homepage "https://github.com/bitbytelabio/vexdom"

  livecheck do
    url :url
    strategy :github_latest
  end

  auto_updates true
  depends_on macos: :sonoma

  app "Vexdom.app"
  binary "#{appdir}/Vexdom.app/Contents/MacOS/vexdom"

  # Ad-hoc signed (no Developer ID): Gatekeeper rejects a quarantined copy.
  postflight_steps do
    run "/usr/bin/xattr", args: ["-dr", "com.apple.quarantine", "{{appdir}}/Vexdom.app"]
  end

  uninstall quit: "com.vexdom.desktop"

  caveats <<~EOS
    Run `vexdom setup` to add the extension to your browser and pair it.
  EOS
end
EOF
