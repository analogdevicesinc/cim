Tutorial: Linux Kernel Cross-Compilation
=========================================

This tutorial builds the
`analogdevicesinc/linux <https://github.com/analogdevicesinc/linux>`_
kernel with a cross-compilation toolchain. You will learn how to use
toolchains with OS/architecture filtering, manifest variables, and
build/clean/flash commands.

What You Will Build
-------------------

A manifest that:

- Downloads the correct ARM cross-compiler for your host OS and architecture
- Clones the ADI Linux kernel
- Cross-compiles the kernel with a single ``make sdk-build``

Step 1: Create the Manifest Structure
--------------------------------------

.. code-block:: bash

   mkdir -p my-manifests/targets/adi-linux

Step 2: Write sdk.yml
---------------------

Create ``my-manifests/targets/adi-linux/sdk.yml``:

.. code-block:: yaml

   mirror: $HOME/tmp/mirror

   variables:
     ARCH: arm64
     CROSS_COMPILE: aarch64-none-linux-gnu-
     KERNEL_DEFCONFIG: adi_defconfig

   toolchains:
     - url: https://developer.arm.com/-/media/Files/downloads/gnu/13.3.rel1/binrel/arm-gnu-toolchain-13.3.rel1-x86_64-aarch64-none-linux-gnu.tar.xz
       destination: toolchains/aarch64
       strip_components: 1
       os: linux
       arch: x86_64

     - url: https://developer.arm.com/-/media/Files/downloads/gnu/13.3.rel1/binrel/arm-gnu-toolchain-13.3.rel1-aarch64-aarch64-none-linux-gnu.tar.xz
       destination: toolchains/aarch64
       strip_components: 1
       os: linux
       arch: aarch64

   envsetup:
     - export PATH=$(pwd)/toolchains/aarch64/bin:$$PATH

   build:
     - >-
       $(MAKE) -C linux
       ARCH=${{ ARCH }}
       CROSS_COMPILE=${{ CROSS_COMPILE }}
       ${{ KERNEL_DEFCONFIG }}
     - >-
       $(MAKE) -C linux
       ARCH=${{ ARCH }}
       CROSS_COMPILE=${{ CROSS_COMPILE }}
       -j$$(nproc)

   clean:
     - >-
       $(MAKE) -C linux
       ARCH=${{ ARCH }}
       CROSS_COMPILE=${{ CROSS_COMPILE }}
       clean

   flash:
     - @echo "Copy linux/arch/${{ ARCH }}/boot/Image to your target"

   gits:
     - name: linux
       url: https://github.com/analogdevicesinc/linux.git
       commit: main

Step 3: Add OS Dependencies
----------------------------

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

   linux-x86_64:
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

Step 4: Initialize and Build
-----------------------------

.. code-block:: bash

   cim init --target adi-linux --source ./my-manifests --install
   cd ~/dsdk-adi-linux
   cim makefile
   make sdk-build

What You Learned
----------------

- **Toolchain filtering.** Two ``toolchains`` entries share the same
  ``destination`` but differ in ``os``/``arch``. CIM picks the right one for
  the host automatically.
- **Variables in build commands.** ``${{ ARCH }}`` becomes ``$(ARCH)`` in
  the generated Makefile, so users can override at build time:
  ``make ARCH=arm sdk-build``.
- **Reproducible builds.** Replace ``commit: main`` with a tag like
  ``2024_r2`` to pin to a specific release.
