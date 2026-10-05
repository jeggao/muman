# muman, built from this checkout. ffmpeg, ffprobe and yt-dlp are set in
# MUMAN_FFMPEG, MUMAN_FFPROBE and MUMAN_YT_DLP with --set-default, so the
# user's own variables still win and hooks see the user's PATH unchanged.
{
  lib,
  rustPlatform,
  makeBinaryWrapper,
  ffmpeg-headless,
  yt-dlp,
  withYtDlp ? true,
}:

let
  manifest = (lib.importTOML ../Cargo.toml).package;
in
rustPlatform.buildRustPackage {
  pname = manifest.name;
  inherit (manifest) version;

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../src
      ../assets
      ../testdata
      ../tests
      ../xtask
    ];
  };

  cargoLock.lockFile = ../Cargo.lock;

  nativeBuildInputs = [ makeBinaryWrapper ];

  # The smoke test generates its audio with the ffmpeg on PATH.
  nativeCheckInputs = [ ffmpeg-headless ];

  cargoBuildFlags = [
    "--package"
    "muman"
  ];

  cargoTestFlags = [
    "--package"
    "muman"
  ];

  checkFlags = [ "--include-ignored" ];

  # muman caches its yt-dlp plugins under the home folder, which the
  # sandbox leaves unwritable.
  preCheck = ''
    export HOME=$(mktemp -d)
  '';

  postInstall = ''
    wrapProgram $out/bin/muman \
      --set-default MUMAN_FFMPEG ${lib.getExe' ffmpeg-headless "ffmpeg"} \
      --set-default MUMAN_FFPROBE ${lib.getExe' ffmpeg-headless "ffprobe"} \
      ${lib.optionalString withYtDlp "--set-default MUMAN_YT_DLP ${lib.getExe yt-dlp}"}
  '';

  meta = {
    inherit (manifest) description;
    homepage = manifest.repository;
    changelog = "${manifest.repository}/blob/v${manifest.version}/CHANGELOG.md";
    license = lib.licenses.agpl3Only;
    mainProgram = "muman";
    platforms = lib.platforms.unix;
  };
}
