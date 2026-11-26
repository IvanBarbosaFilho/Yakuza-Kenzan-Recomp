#!/usr/bin/env bash
# Builds Kenzan-x86_64.AppImage: the launcher, the engine, the three game modules (30/60/120),
# the patched EBOOTs, 7-Zip, and every library they load -- glibc included, started through the
# bundle's own ld-linux so the host's glibc version does not matter. The GPU stack (libGL/libEGL/
# libvulkan and the drivers) is the host's.
#   nix-shell packaging/linux/shell.nix --run 'bash packaging/linux/build_appimage.sh'
set -euo pipefail
D=$(cd "$(dirname "$0")/../.." && pwd)
W=$D/work
OUT=$W/appimage
APP=$OUT/AppDir
rm -rf "$APP"; mkdir -p "$APP/usr/bin" "$APP/usr/lib" "$APP/usr/libexec" "$APP/usr/share/kenzan" "$APP/usr/share/licenses"

echo "== launcher"
( cd "$D/launcher" && cargo build --release 2>&1 | tail -1 )
cp "$D/launcher/target/release/kenzan-launcher" "$APP/usr/bin/"

echo "== engine, modules, executables"
cp "$W/build/KenzanRecomp" "$APP/usr/bin/"
for m in libInfamous2Game.so libInfamous2Game_60.so libInfamous2Game_120.so; do cp "$W/build/$m" "$APP/usr/share/kenzan/"; done
cp "$W/EBOOT_60.elf" "$W/EBOOT_120.elf" "$APP/usr/share/kenzan/"
# shaders translated before release (tools/make_shader_seed.py; engine PS3RECOMP_VK_SHADER_SEED)
# the Japanese original (launcher lang.rs: the Home page's language switch): its executables and
# game modules (work/jp, lifted from the BLJM60064 EBOOT with Pakundo's JP-hash FPS patches)
# (only in the Japanese edition, Kenzan-Japanese-x86_64.AppImage: staged here, added at the end)
JPS=$OUT/japanese-stage; rm -rf "$JPS"; mkdir -p "$JPS"
for f in EBOOT.elf EBOOT_60.elf EBOOT_120.elf build/libInfamous2Game.so build/libInfamous2Game_60.so build/libInfamous2Game_120.so; do
  cp "$W/jp/$f" "$JPS/" || { echo "missing $W/jp/$f"; exit 1; }
done
[ -f "$W/shader_seed/seed.lst" ] && cp -r "$W/shader_seed" "$APP/usr/share/kenzan/shader_seed"
SEVEN=$(command -v 7zz); cp "$SEVEN" "$APP/usr/libexec/7zz"

echo "== libraries"
# the libraries the programs link, plus the ones SDL (and sdl2-compat -> SDL3) and the launcher's
# window code dlopen at run time
DLOPEN="libSDL3.so.0 libX11.so.6 libX11-xcb.so.1 libxcb.so.1 libXext.so.6 libXcursor.so.1 libXi.so.6 libXrandr.so.2
        libXfixes.so.3 libXrender.so.1 libXss.so.1 libXinerama.so.1 libxkbcommon.so.0 libwayland-client.so.0
        libwayland-cursor.so.0 libwayland-egl.so.1 libdecor-0.so.0 libpulse.so.0 libasound.so.2 libdbus-1.so.3"
# the host's own: graphics drivers and their loaders
SKIP='^(libGL\.|libEGL\.|libGLX|libGLdispatch|libGLES|libvulkan\.|libdrm|libgbm|libnvidia|linux-vdso|ld-linux)'
find_lib() {   # a library by soname, from the nix-shell's LD_LIBRARY_PATH
  local IFS=:; for d in $LD_LIBRARY_PATH; do [ -e "$d/$1" ] && { readlink -f "$d/$1"; return; }; done
}
declare -A seen
add_with_deps() {   # $1 = a file; copies every library it needs (ldd resolves through the nix paths)
  ldd "$1" 2>/dev/null | awk '/=> \// {print $1, $3}' | while read -r name path; do
    name=${name##*/}
    [[ $name =~ $SKIP ]] && continue
    [ -e "$APP/usr/lib/$name" ] && continue
    cp -L "$path" "$APP/usr/lib/$name"
  done
}
for f in "$APP/usr/bin/KenzanRecomp" "$APP/usr/bin/kenzan-launcher" "$APP/usr/libexec/7zz"; do add_with_deps "$f"; done
for so in $DLOPEN; do
  p=$(find_lib "$so") || true
  if [ -z "${p:-}" ]; then echo "   (not found, skipped: $so)"; continue; fi
  [ -e "$APP/usr/lib/$so" ] || cp -L "$p" "$APP/usr/lib/$so"
  add_with_deps "$p"
done
# a second pass: dependencies of the dlopen'd libraries' own dependencies
for f in "$APP"/usr/lib/*.so*; do add_with_deps "$f"; done
# the dynamic loader that goes with this glibc
LDSO=$(ldd "$APP/usr/bin/KenzanRecomp" | awk '/ld-linux-x86-64/ {print $1}')
cp -L "$LDSO" "$APP/usr/lib/ld-linux-x86-64.so.2"
chmod +x "$APP/usr/lib/ld-linux-x86-64.so.2"
echo "   $(ls "$APP/usr/lib" | wc -l) libraries"

echo "== launchers"
cat > "$APP/AppRun" <<'EOF'
#!/bin/bash
# Everything runs through the bundle's ld-linux with the bundle's libraries first and the host's
# library folders after them (for the GPU drivers and their GL/Vulkan loaders).
HERE=$(dirname "$(readlink -f "$0")")
export KZ_APPDIR="$HERE"
export KZ_LDSO="$HERE/usr/lib/ld-linux-x86-64.so.2"
HOST=/usr/lib/x86_64-linux-gnu:/lib/x86_64-linux-gnu:/usr/lib64:/lib64:/usr/lib:/lib:/run/opengl-driver/lib
export KZ_LIBPATH="$HERE/usr/lib:$HOST"
export PATH="$HERE/usr/bin:$PATH"
exec "$KZ_LDSO" --library-path "$KZ_LIBPATH" "$HERE/usr/bin/kenzan-launcher" "$@"
EOF
cat > "$APP/usr/bin/7zz" <<'EOF'
#!/bin/bash
exec "$KZ_LDSO" --library-path "$KZ_LIBPATH" "$KZ_APPDIR/usr/libexec/7zz" "$@"
EOF
chmod +x "$APP/AppRun" "$APP/usr/bin/7zz"

echo "== desktop entry, icon, licenses"
cat > "$APP/kenzan.desktop" <<'EOF'
[Desktop Entry]
Type=Application
Name=Ryu ga Gotoku Kenzan! (PC port)
Comment=Launcher and settings for the Ryu ga Gotoku Kenzan! PC port
Exec=kenzan-launcher
Icon=kenzan
Categories=Game;
Terminal=false
EOF
cp "$D/packaging/linux/kenzan.png" "$APP/kenzan.png"
ln -sf kenzan.png "$APP/.DirIcon"
cp "$D/launcher/assets/OFL-ShipporiMincho.txt" "$APP/usr/share/licenses/"

echo "== AppImage"
RUNTIME=$OUT/runtime-x86_64
[ -s "$RUNTIME" ] || curl -sSL -o "$RUNTIME" https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-x86_64
pack() {   # <AppImage name>
  rm -f "$OUT/kenzan.squashfs" "$OUT/$1"
  mksquashfs "$APP" "$OUT/kenzan.squashfs" -root-owned -noappend -comp zstd -Xcompression-level 15 -quiet
  cat "$RUNTIME" "$OUT/kenzan.squashfs" > "$OUT/$1"
  chmod +x "$OUT/$1"
  rm -f "$OUT/kenzan.squashfs"
  ls -la "$OUT/$1"
}
rm -rf "$APP/usr/share/kenzan/japanese"
pack Kenzan-x86_64.AppImage
mv "$JPS" "$APP/usr/share/kenzan/japanese"
pack Kenzan-Japanese-x86_64.AppImage
