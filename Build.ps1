$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
cargo build --manifest-path "$root/native/Cargo.toml" --target-dir "$root/native/target" --release --locked -j 2
if ($LASTEXITCODE -ne 0) { throw 'MLSB build failed' }
Copy-Item -LiteralPath "$root/native/target/release/mlsb.exe" -Destination "$root/MLSB.exe" -Force
Write-Host 'Built MLSB.exe.'
