# Packaging environment: the game's and the launcher's run-time libraries (so ldd resolves them
# all), the libraries SDL/the launcher dlopen, and the tools to build the AppImage.
{ pkgs ? import <nixpkgs> {} }:
let
  libs = with pkgs; [ SDL2 sdl3 vulkan-loader spirv-cross glslang spirv-tools ffmpeg.lib zlib stdenv.cc.cc.lib
                      libGL libxkbcommon wayland libx11 libxext libxcursor libxrandr libxi libxfixes libxrender
                      libxscrnsaver libxinerama libxcb libdecor libpulseaudio alsa-lib dbus ];
in
pkgs.mkShell {
  nativeBuildInputs = with pkgs; [ rustc cargo pkg-config _7zz squashfsTools curl imagemagick ];
  buildInputs = libs;
  shellHook = ''
    export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath libs}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}
  '';
}
