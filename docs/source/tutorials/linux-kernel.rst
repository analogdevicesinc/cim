Tutorial: Linux Kernel Cross-Compilation
========================================

This tutorial builds the
`analogdevicesinc/linux <https://github.com/analogdevicesinc/linux>`_
kernel with a cross-compilation toolchain. You will learn how to use
toolchains with OS/architecture filtering, manifest variables, and
build/clean/flash commands.

What You Will Build
-------------------

A manifest that:

- Downloads the Arm GNU cross-compiler on x86_64 Linux hosts
- Clones the ADI Linux kernel
- Cross-compiles the arm64 kernel with a single ``make sdk-build``

Step 1: Create the Manifest Structure
-------------------------------------

.. code-block:: bash

   mkdir -p my-manifests/targets/adi-linux

Step 2: Write sdk.yml
---------------------

Create ``my-manifests/targets/adi-linux/sdk.yml``:

.. code-block:: yaml

   variables:
     ARCH: arm64
     CROSS_COMPILE: ${{ WORKSPACE }}/toolchains/aarch64/bin/aarch64-none-linux-gnu-
     KERNEL_DEFCONFIG: adi_zynqmp_defconfig

   toolchains:
     - url: https://developer.arm.com/-/media/Files/downloads/gnu/13.3.rel1/binrel/arm-gnu-toolchain-13.3.rel1-x86_64-aarch64-none-linux-gnu.tar.xz
       destination: toolchains/aarch64
       strip_components: 1
       os: linux
       arch: x86_64
       sha256: 322f0b4482fc0d9fa0bb468134841f08d8c554c54ff5aa29a13a7a24bf7e1eb5

   build:
     commands: |
       $(MAKE) -C linux ARCH=${{ ARCH }} CROSS_COMPILE=${{ CROSS_COMPILE }} ${{ KERNEL_DEFCONFIG }}
       $(MAKE) -C linux ARCH=${{ ARCH }} CROSS_COMPILE=${{ CROSS_COMPILE }} Image

   clean:
     commands: |
       $(MAKE) -C linux ARCH=${{ ARCH }} CROSS_COMPILE=${{ CROSS_COMPILE }} clean

   flash:
     commands: |
       @echo "Copy linux/arch/${{ ARCH }}/boot/Image to your target"

   gits:
     - name: linux
       url: https://github.com/analogdevicesinc/linux.git
       commit: main

A few things to note:

- ``CROSS_COMPILE`` is the full path prefix of the compiler in the workspace.
  Each build command runs in its own shell, so the manifest passes the
  location explicitly instead of changing ``PATH``.
- Each line of a ``commands`` block is one command. ``@`` at the start of a
  line stops Make from echoing it. Inside a ``|`` block it needs no quoting;
  in a YAML list it must be quoted (``- "@echo ..."``), because ``@`` cannot
  start a plain YAML value.
- The build passes no ``-j`` flag. You choose the parallelism when you run
  ``make``, and ``$(MAKE)`` passes it on to the kernel build.

Step 3: Add OS Dependencies
---------------------------

Create ``my-manifests/targets/adi-linux/os-dependencies.yml``:

.. code-block:: yaml

   linux_kernel_deps: &kernel_deps
     - bc
     - bison
     - build-essential
     - flex
     - libelf-dev
     - libncurses-dev
     - libssl-dev
     - lz4

   linux:
     ubuntu-22.04:
       command: "apt-get install"
       packages: *kernel_deps
     ubuntu-24.04:
       command: "apt-get install"
       packages: *kernel_deps
     fedora-42:
       command: "dnf install"
       packages:
         - bc
         - bison
         - elfutils-libelf-devel
         - flex
         - gcc
         - make
         - ncurses-devel
         - openssl-devel
         - lz4

The ``linux`` key applies to every Linux host architecture; use
``linux-x86_64`` or ``linux-aarch64`` when the lists differ.

Step 4: Initialize and Build
----------------------------

.. code-block:: bash

   cim init --target adi-linux --source ./my-manifests --full
   cd ~/dsdk-adi-linux
   make -j$(nproc) sdk-build

``--full`` installs the OS packages (asking for sudo), downloads the
toolchain, and generates the Makefile. The first clone of the kernel is large;
later workspaces clone from the local mirror.

What You Learned
----------------

- **Toolchain filtering.** The ``toolchains`` entry is limited to
  ``os: linux`` and ``arch: x86_64``, so CIM installs it only on such hosts. To
  support more hosts, add entries with the same ``destination`` and other
  ``os``/``arch`` values. On an arm64 Linux host the kernel can be built with
  the native compiler: ``make CROSS_COMPILE= sdk-build``.
- **Variables in build commands.** ``${{ KERNEL_DEFCONFIG }}`` becomes
  ``$(KERNEL_DEFCONFIG)`` in the generated Makefile, so users can override it
  at build time: ``make KERNEL_DEFCONFIG=adi_versal_defconfig sdk-build``.
- **Reproducible builds.** Replace ``commit: main`` with a release tag like
  ``2023_r2`` to pin the kernel to a specific release.
