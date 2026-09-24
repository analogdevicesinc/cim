How to Manage Toolchains
========================

This guide covers adding, filtering, and configuring toolchains in your
manifest.

Add a Toolchain
---------------

Add entries to the ``toolchains`` section in ``sdk.yml``:

.. code-block:: yaml

   toolchains:
     - url: https://developer.arm.com/.../arm-gnu-toolchain.tar.xz
       destination: toolchains/aarch64
       strip_components: 1
       sha256: e4cea5bb6...

Install toolchains with:

.. code-block:: bash

   cim install toolchains

Filter by OS and Architecture
------------------------------

Provide multiple entries with the same ``destination`` but different
``os``/``arch`` filters. CIM selects the matching entry for the host:

.. code-block:: yaml

   toolchains:
     - url: https://example.com/toolchain-linux-x86_64.tar.xz
       destination: toolchains/arm
       os: linux
       arch: x86_64

     - url: https://example.com/toolchain-linux-aarch64.tar.xz
       destination: toolchains/arm
       os: linux
       arch: aarch64

     - url: https://example.com/toolchain-macos-arm64.tar.xz
       destination: toolchains/arm
       os: darwin
       arch: arm64

Run Post-Install Commands
--------------------------

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
         - "bash ./sh.rustup.rs -y --no-modify-path"
         - "rustup toolchain install nightly-2025-01-01"

``$PWD`` expands to the toolchain installation directory,
``$WORKSPACE`` to the workspace root, and ``$HOME`` to the user's home.

Use Symlinks and Mirrors
-------------------------

To save disk space when multiple workspaces share toolchains:

.. code-block:: bash

   cim install toolchains --symlink

This stores the toolchain in the mirror and creates symlinks in the
workspace. Use ``--force`` to re-download even if the mirror copy exists.

Verify Integrity with Checksums
--------------------------------

Add ``sha256`` to toolchain entries for integrity verification:

.. code-block:: yaml

   toolchains:
     - url: https://example.com/toolchain.tar.xz
       destination: toolchains/arm
       sha256: 65d1191f755c92d6b7792b1d054cbd3aa6762bb2...

If the checksum does not match, the archive is re-downloaded (up to 3
attempts before reporting an error).

To compute checksums for toolchains already in the mirror:

.. code-block:: bash

   cim utils hash-toolchains --add-missing
