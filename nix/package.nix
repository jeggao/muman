# muman, built from this checkout. ffmpeg, ffprobe, yt-dlp and aria2c are
# set in MUMAN_FFMPEG, MUMAN_FFPROBE, MUMAN_YT_DLP and MUMAN_ARIA2C with
# --set-default, so the user's own variables still win and hooks see the
# user's PATH unchanged.
{
  lib,
  rustPlatform,
  makeBinaryWrapper,
  ffmpeg-headless,
  yt-dlp,
  aria2,
  withYtDlp ? true,
  withAria2 ? true,
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

  # The smoke test generates its audio with the ffmpeg on PATH, the
  # yt-dlp test hands yt-dlp muman's command line, and the torrent test
  # aria2c its own, against a swarm on the loopback.
  nativeCheckInputs = [
    ffmpeg-headless
    yt-dlp
    aria2
  ];

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
      ${lib.optionalString withYtDlp "--set-default MUMAN_YT_DLP ${lib.getExe yt-dlp}"} \
      ${lib.optionalString withAria2 "--set-default MUMAN_ARIA2C ${lib.getExe' aria2 "aria2c"}"}
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
