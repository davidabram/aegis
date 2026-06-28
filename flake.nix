{
  description = "Aegis development and package flake";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
      lib = pkgs.lib;

      cefVersion = "146.0.6+g68649e2+chromium-146.0.7680.154";
      cefBinary = "cef_binary_${cefVersion}_linux64";
      cefSdk = pkgs.fetchzip {
        url = "https://cef-builds.spotifycdn.com/${builtins.replaceStrings [ "+" ] [ "%2B" ] cefBinary}.tar.bz2";
        hash = "sha256-qnHJnqONKH/6r1+tIcN8pG+qGBB3NgJ3zNA2eXicv5M=";
      };

      runtimeLibs = with pkgs; [
        alsa-lib
        atk
        at-spi2-atk
        at-spi2-core
        cairo
        cups
        dbus
        expat
        glib
        gtk3
        libdrm
        libgbm
        libGL
        libxkbcommon
        mesa
        nspr
        nss
        pango
        udev
        libx11
        libxcb
        libxcomposite
        libxcursor
        libxdamage
        libxext
        libxfixes
        libxi
        libxrandr
        libxrender
        libxscrnsaver
        libxtst
      ];

      libraryPath = lib.makeLibraryPath runtimeLibs;

      devAegis = pkgs.writeShellScriptBin "aegis" ''
        set -euo pipefail
        export AEGIS_CEF_ROOT="${cefSdk}"
        export CEF_ROOT="${cefSdk}"
        export LD_LIBRARY_PATH="${libraryPath}:''${LD_LIBRARY_PATH:-}"

        if [[ "$#" -eq 0 ]]; then
          exec cargo run -- --mode headful serve
        fi

        if [[ "''${1:-}" == "open" ]]; then
          shift
          exec cargo run -- --mode headful serve "$@"
        fi

        exec cargo run -- "$@"
      '';

      aegis = pkgs.rustPlatform.buildRustPackage {
        pname = "aegis";
        version = "0.1.0";
        src = self;

        cargoLock.lockFile = ./Cargo.lock;

        nativeBuildInputs = with pkgs; [
          autoPatchelfHook
          cmake
          makeWrapper
          pkg-config
          python3
        ];

        buildInputs = runtimeLibs;

        AEGIS_CEF_ROOT = cefSdk;
        CEF_ROOT = cefSdk;

        preBuild = ''
          cmake -S native -B native/build/linux \
            -DAEGIS_TARGET_PLATFORM=linux \
            -DCEF_ROOT=${cefSdk} \
            -DCMAKE_BUILD_TYPE=Release
          cmake --build native/build/linux --target aegis_native --parallel "$NIX_BUILD_CORES"
        '';

        postInstall = ''
          native_out="native/build/linux/release"

          mv "$out/bin/aegis" "$out/bin/aegis_cli"
          install -Dm755 "$native_out/aegis_native" "$out/bin/aegis_native"
          install -Dm755 "$native_out/aegis_helper" "$out/lib/aegis_helper"
          install -Dm755 "$native_out/libaegis_host.so" "$out/lib/libaegis_host.so"

          for file in \
            libcef.so \
            chrome-sandbox \
            libEGL.so \
            libGLESv2.so \
            libvk_swiftshader.so \
            libvulkan.so.1 \
            icudtl.dat \
            snapshot_blob.bin \
            v8_context_snapshot.bin \
            vk_swiftshader_icd.json \
            chrome_100_percent.pak \
            chrome_200_percent.pak \
            resources.pak; do
            if [[ -e "$native_out/$file" ]]; then
              install -Dm755 "$native_out/$file" "$out/lib/$file"
            fi
          done

          for dir in locales swiftshader; do
            if [[ -d "$native_out/$dir" ]]; then
              cp -R "$native_out/$dir" "$out/lib/$dir"
            fi
          done

          cat > "$out/bin/aegis" <<EOF
#!${pkgs.runtimeShell}
set -euo pipefail
export AEGIS_CEF_ROOT="${cefSdk}"
export CEF_ROOT="${cefSdk}"
export LD_LIBRARY_PATH="$out/lib:${libraryPath}:\''${LD_LIBRARY_PATH:-}"

has_host_lib=0
for arg in "\$@"; do
  case "\$arg" in
    --host-lib|--host-lib=*) has_host_lib=1 ;;
  esac
done

if [[ "\$#" -eq 0 ]]; then
  exec "$out/bin/aegis_cli" --host-lib "$out/lib/libaegis_host.so" --mode headful serve
fi

if [[ "''${1:-}" == "open" ]]; then
  shift
  exec "$out/bin/aegis_cli" --host-lib "$out/lib/libaegis_host.so" --mode headful serve "\$@"
fi

if [[ "\$has_host_lib" -eq 1 ]]; then
  exec "$out/bin/aegis_cli" "\$@"
fi

exec "$out/bin/aegis_cli" --host-lib "$out/lib/libaegis_host.so" "\$@"
EOF
          chmod +x "$out/bin/aegis"

          wrapProgram "$out/bin/aegis_cli" \
            --set AEGIS_CEF_ROOT "${cefSdk}" \
            --set CEF_ROOT "${cefSdk}" \
            --prefix LD_LIBRARY_PATH : "$out/lib:${libraryPath}"
        '';

        meta = {
          description = "Agentic web browser CLI and runtime control plane";
          platforms = [ system ];
        };
      };
    in
    {
      packages.${system} = {
        default = aegis;
        aegis = aegis;
      };

      apps.${system}.default = {
        type = "app";
        program = "${aegis}/bin/aegis";
      };

      checks.${system}.default = aegis;

      devShells.${system}.default = pkgs.mkShell {
        packages = with pkgs; [
          cargo
          clang
          clippy
          cmake
          devAegis
          pkg-config
          python3
          rustc
          rustfmt
        ] ++ runtimeLibs;

        AEGIS_CEF_ROOT = cefSdk;
        CEF_ROOT = cefSdk;
        LD_LIBRARY_PATH = "${libraryPath}";

        shellHook = ''
          echo "Aegis dev shell"
          echo "CEF_ROOT=${cefSdk}"
          echo "Run: aegis native doctor"
        '';
      };
    };
}
