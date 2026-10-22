# Homebrew cask. Lives in a tap (github.com/kidchenko/homebrew-tap) as
# Casks/porta.rb, then: brew install --cask kidchenko/tap/porta
#
# The release workflow rewrites `version` and `sha256` from the DMG it built.
cask "porta" do
  version "0.0.1"
  sha256 "REPLACE_WITH_SHA256"

  url "https://github.com/kidchenko/porta/releases/download/v#{version}/Porta-#{version}.dmg"
  name "Porta"
  desc "Menu bar list of what is listening on your ports"
  homepage "https://github.com/kidchenko/porta"

  depends_on macos: ">= :big_sur"

  app "Porta.app"

  # Ad-hoc signed, not notarized: without this, Gatekeeper refuses to open it.
  postflight do
    system_command "/usr/bin/xattr", args: ["-dr", "com.apple.quarantine", "#{appdir}/Porta.app"]
  end

  uninstall quit:      "com.kidchenko.porta",
            launchctl: "com.kidchenko.porta"

  zap trash: [
    "~/Library/Application Support/Porta",
    "~/Library/LaunchAgents/com.kidchenko.porta.plist",
  ]
end
