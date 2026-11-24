# Build + run environment for the Kenzan launcher (Rust + egui/eframe).
#   nix-shell launcher/shell.nix --run 'cargo run --release'
{ pkgs ? import <nixpkgs> {} }:
let
  runtimeLibs = with pkgs; [ libGL libxkbcommon wayland libx11 libxcursor libxrandr libxi vulkan-loader ];
in
pkgs.mkShell {
  nativeBuildInputs = with pkgs; [ rustc cargo pkg-config _7zz ];
  buildInputs = runtimeLibs;
  shellHook = ''
    export LD_LIBRARY_PATH=/run/opengl-driver/lib:${pkgs.lib.makeLibraryPath runtimeLibs}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}
  '';
}
