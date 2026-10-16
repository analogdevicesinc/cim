How to Manage Toolchains
========================

This guide covers adding, filtering, and configuring toolchains in your
manifest.

Add a Toolchain
---------------

Add entries to the ``toolchains`` section in ``sdk.yml``:

.. code-block:: yaml

   toolchains:
     - url: https://developer.arm.com/-/media/Files/downloads/gnu/13.3.rel1/binrel/arm-gnu-toolchain-13.3.rel1-x86_64-aarch64-none-linux-gnu.tar.xz
       destination: toolchains/aarch64
       strip_components: 1
       os: linux
       arch: x86_64

Install toolchains with:

.. code-block:: bash

   cim install toolchains

The archive is downloaded to the mirror and extracted to
``<workspace>/toolchains/aarch64``. ``strip_components: 1`` drops the
archive's top-level directory, so the compiler ends up in
``toolchains/aarch64/bin/aarch64-none-linux-gnu-gcc``. Reference it from build
commands with ``${{ WORKSPACE }}/toolchains/aarch64/bin/...``.

``cim init --install`` and ``cim init --full`` install toolchains as part of
initialization.

Filter by OS and Architecture
-----------------------------

Provide multiple entries with the same ``destination`` but different
``os``/``arch`` filters. CIM installs only the entries that match the host:

.. code-block:: yaml

   toolchains:
     - url: https://developer.arm.com/-/media/Files/downloads/gnu/13.3.rel1/binrel/arm-gnu-toolchain-13.3.rel1-x86_64-aarch64-none-elf.tar.xz
       destination: toolchains/aarch64-none-elf
       strip_components: 1
       os: linux
       arch: x86_64

     - url: https://developer.arm.com/-/media/Files/downloads/gnu/13.3.rel1/binrel/arm-gnu-toolchain-13.3.rel1-aarch64-aarch64-none-elf.tar.xz
       destination: toolchains/aarch64-none-elf
       strip_components: 1
       os: linux
       arch: arm64

     - url: https://developer.arm.com/-/media/Files/downloads/gnu/13.3.rel1/binrel/arm-gnu-toolchain-13.3.rel1-darwin-arm64-aarch64-none-elf.tar.xz
       destination: toolchains/aarch64-none-elf
       strip_components: 1
       os: darwin
       arch: arm64

Valid ``os`` values are ``linux``, ``darwin`` and ``windows``. Valid ``arch``
values are ``x86_64``, ``arm64`` and ``i386``. 64-bit Arm hosts are
``arm64`` on both Linux and macOS (not ``aarch64``). Entries without ``os`` or
``arch`` match every host.

Run Post-Install Commands
-------------------------

Use ``post_install_commands`` to run setup steps after extraction. The
``environment`` field isolates the toolchain from system-wide installations:

.. code-block:: yaml

   toolchains:
     - url: https://sh.rustup.rs
       destination: toolchains/rust
       environment:
         CARGO_HOME: "$PWD/cargo"
         RUSTUP_HOME: "$PWD/rustup"
         PATH: "$PWD/cargo/bin:$PATH"
       post_install_commands:
         - "mkdir -p cargo rustup"
         - "bash ./sh.rustup.rs -y --no-modify-path --profile minimal --default-toolchain none"
         - "rustup toolchain install 1.85.0 --profile minimal"
         - "rustup default 1.85.0"

The commands run in ``bash`` with the toolchain directory as the working
directory. ``$PWD`` expands to the toolchain installation directory,
``$WORKSPACE`` to the workspace root, and ``$HOME`` to the user's home.
Non-archive downloads such as the ``rustup`` installer script are copied into
the destination as-is.

Use Symlinks and Mirrors
------------------------

To save disk space when multiple workspaces share toolchains:

.. code-block:: bash

   cim install toolchains --symlink

This extracts the toolchain into the mirror and creates symlinks in the
workspace. Use ``--force`` to remove the existing destination and install
again.

Verify Integrity with Checksums
-------------------------------

Add ``sha256`` to toolchain entries for integrity verification:

.. code-block:: yaml

   toolchains:
     - url: https://developer.arm.com/-/media/Files/downloads/gnu/13.3.rel1/binrel/arm-gnu-toolchain-13.3.rel1-x86_64-aarch64-none-linux-gnu.tar.xz
       destination: toolchains/aarch64
       strip_components: 1
       os: linux
       arch: x86_64
       sha256: 322f0b4482fc0d9fa0bb468134841f08d8c554c54ff5aa29a13a7a24bf7e1eb5

If the checksum does not match, the archive is re-downloaded (up to 3
attempts before reporting an error).

To compute checksums for toolchains that are already downloaded to the mirror,
run this inside the workspace:

.. code-block:: bash

   cim utils hash-toolchains --add-missing

It updates the workspace ``sdk.yml``; copy the new ``sha256`` lines back to
your manifest repository. Entries are matched by their ``name`` field, so give
the entries you want hashed an explicit ``name`` (the archive file name, e.g.
``name: arm-gnu-toolchain-13.3.rel1-x86_64-aarch64-none-linux-gnu.tar.xz``).
Entries without one are reported as skipped, with the computed hash printed in
``--verbose`` mode.

Authenticated Downloads
-----------------------

For archives behind authentication, add ``headers`` or ``basic_auth``. Values
may reference host environment variables, which must be set before running
``cim install toolchains``; CIM never stores the expanded secret:

.. code-block:: yaml

   toolchains:
     - url: https://artifacts.example.com/protected/vendor-toolchain.tar.gz
       destination: toolchains/vendor
       headers:
         Authorization: "Bearer $MY_API_TOKEN"

``basic_auth: "user:$MY_PASSWORD"`` works the same way for HTTP Basic auth.
The URL above is a placeholder for your own artifact server.
