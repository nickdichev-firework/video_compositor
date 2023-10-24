{ pkgs, ... }:

{
  packages = [
    pkgs.git
    pkgs.ffmpeg_6-full
    pkgs.pkg-config
    pkgs.openssl
    pkgs.cups
    pkgs.jansson
    pkgs.nss
    pkgs.nspr
    pkgs.at-spi2-core
    pkgs.llvmPackages.bintools
  ];

  languages.rust.enable = true;

  env.LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
}
