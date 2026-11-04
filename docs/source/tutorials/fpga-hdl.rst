Tutorial: FPGA HDL Designs
==========================

This tutorial builds
`analogdevicesinc/hdl <https://github.com/analogdevicesinc/hdl>`_
reference designs with AMD Vivado. You will learn how to integrate external
vendor tools, ship a build wrapper with ``copy_files``, guard a build with
``envsetup``, and select projects with variables.

What You Will Build
-------------------

A manifest that:

- Uses a Vivado installation that already exists on the host (no
  ``toolchains`` section)
- Ships a wrapper script that loads the Vivado environment and runs the HDL
  build
- Builds HDL designs with configurable project and board variables

Vivado must be installed separately. The ``main`` branch of the HDL repository
requires the Vivado version set in ``hdl/scripts/adi_env.tcl`` (2025.1 at the
time of writing); release branches such as ``hdl_2023_r2`` work with older
versions.

Step 1: Create the Manifest Structure
-------------------------------------

.. code-block:: bash

   mkdir -p my-manifests/targets/adi-hdl

Step 2: Write sdk.yml
---------------------

Create ``my-manifests/targets/adi-hdl/sdk.yml``:

.. code-block:: yaml

   variables:
     HDL_PROJECT: fmcomms2
     HDL_BOARD: zed
     VIVADO_DIR: /tools/Xilinx/2025.1/Vivado

   copy_files:
     - source: build-hdl.sh
       dest: scripts/build-hdl.sh

   envsetup:
     commands: |
       @test -f ${{ VIVADO_DIR }}/settings64.sh || { echo "Vivado not found in ${{ VIVADO_DIR }}, set VIVADO_DIR"; exit 1; }

   build:
     commands: |
       +bash scripts/build-hdl.sh ${{ VIVADO_DIR }} hdl/projects/${{ HDL_PROJECT }}/${{ HDL_BOARD }}
     depends_on:
       - sdk-envsetup

   clean:
     commands: |
       bash scripts/build-hdl.sh ${{ VIVADO_DIR }} hdl/projects/${{ HDL_PROJECT }}/${{ HDL_BOARD }} clean

   gits:
     - name: hdl
       url: https://github.com/analogdevicesinc/hdl.git
       commit: main

``VIVADO_DIR`` is the directory that contains Vivado's ``settings64.sh``
(here a Vivado 2025.1 installed under ``/tools/Xilinx``).
``envsetup`` stops the build early with a clear message when it is wrong.

The leading ``+`` marks the build command as recursive, so Make shares its
job slots with the HDL build: ``make -j8 sdk-build`` builds the HDL IP
libraries in parallel.

Step 3: Add the Build Wrapper
-----------------------------

Every command in the generated Makefile runs in its own shell, so sourcing
``settings64.sh`` in one command has no effect on the next one. The wrapper
loads the Vivado environment and runs the HDL build in the same shell. Create
``my-manifests/targets/adi-hdl/build-hdl.sh``:

.. code-block:: bash

   #!/bin/bash
   # Build an HDL project with Vivado's environment loaded.
   # Usage: build-hdl.sh <vivado-install-dir> <project-dir> [make-target...]
   set -e
   vivado_dir=$1
   project_dir=$2
   shift 2
   targets=("$@")
   set --
   . "$vivado_dir/settings64.sh"
   # Run the HDL build as a top-level make: keep the -j/jobserver flags of the
   # workspace make, but drop its variable overrides (HDL_PROJECT=... and so on).
   MAKEFLAGS=${MAKEFLAGS%%-- *}
   unset MAKEOVERRIDES MAKELEVEL
   exec make -C "$project_dir" "${targets[@]}"

The HDL build scripts expect to run as the top-level ``make``. Nested inside
the workspace ``make``, some projects (those using the transceiver wizard,
such as the JESD204 designs) compute directory names differently, and
variables set on the workspace ``make`` command line would reach the HDL build
as extra build parameters. The wrapper removes both before it starts the HDL
build. ``set --`` clears the script arguments so that ``settings64.sh`` does
not see them.

``copy_files`` resolves the relative ``source: build-hdl.sh`` against the
target directory and copies the file into the workspace during ``cim init``.

Step 4: Initialize and Build
----------------------------

.. code-block:: bash

   cim init --target adi-hdl --source ./my-manifests --install
   cd ~/dsdk-adi-hdl
   make -j8 sdk-build

If Vivado is installed elsewhere, or to target a different board without
editing the manifest, set the variables on the command line:

.. code-block:: bash

   make -j8 VIVADO_DIR=/opt/Xilinx/2025.1/Vivado sdk-build
   make -j8 HDL_PROJECT=ad9081_fmca_ebz HDL_BOARD=zcu102 sdk-build

What You Learned
----------------

- **External vendor tools.** Vivado and Quartus are large tools installed
  separately. A wrapper script loads their environment and runs the build in
  one shell, so users do not need to set up their shell first.
- **Explicit dependencies.** ``depends_on: [sdk-envsetup]`` makes
  ``make sdk-build`` check for Vivado first.
- **copy_files for helper files.** Scripts and other files in the manifest
  repository can be copied into every workspace; remote files can be
  downloaded and cached in the mirror the same way.
- **Variables for project selection.** ``HDL_PROJECT``, ``HDL_BOARD`` and
  ``VIVADO_DIR`` let users target different boards and Vivado installations
  at build time without editing the manifest.
