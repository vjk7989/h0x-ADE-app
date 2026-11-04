Captured on macOS with `/usr/bin/codesign -dvv HyperOx.app` from the published 0.1.8 macos-aarch64 tarball (SHA-256 `adc225fe8b6826737357ba7065d72e6ab7bb583e568a57d0e9bf48506f2db0a2`). All output was on stderr; stdout was empty.

- `developer-id.txt`: original release, exit 0.
- `adhoc.txt`: after `codesign --force --sign - HyperOx.app`, exit 0.
- `unsigned.txt`: after `codesign --remove-signature HyperOx.app`, exit 1.
