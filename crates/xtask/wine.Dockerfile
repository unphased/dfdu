# The image `cargo xtask windows --test` runs the workspace's Windows tests in: Rust with the
# x86_64-pc-windows-gnu target, mingw-w64 to link it, Wine to run it, and cargo-nextest.
FROM --platform=linux/amd64 rust:1.100
RUN dpkg --add-architecture i386 && apt-get -qq update \
 && apt-get -qq install -y --no-install-recommends mingw-w64 wine wine64 >/dev/null \
 && rm -rf /var/lib/apt/lists/*
# rust-toolchain.toml asks for `stable`, which needs the target too.
RUN rustup toolchain install stable --profile minimal -c rustfmt,clippy -t x86_64-pc-windows-gnu \
 && rustup default stable
RUN curl -LsSf https://get.nexte.st/latest/linux | tar zxf - -C /usr/local/cargo/bin
ENV WINEPREFIX=/wine WINEDEBUG=-all CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER=wine
RUN wineboot --init >/dev/null 2>&1 || true
