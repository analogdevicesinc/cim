Tutorial: FPGA HDL Designs
==========================

This tutorial builds
`analogdevicesinc/hdl <https://github.com/analogdevicesinc/hdl>`_
reference designs with AMD Vivado. You will learn how to integrate external
vendor tools, guard a build with ``envsetup``, ship a helper script with
``copy_files``, and select projects with variables.

What You Will Build
-------------------

A manifest that:

- Uses a Vivado installation that already exists on the host (no
  ``toolchains`` section)
- Ships an ``env.sh`` script that loads the Vivado environment
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

   copy_files:
     - source: env.sh
       dest: env.sh

   envsetup:
     commands: |
       @command -v vivado >/dev/null || { echo "vivado not found, run: . ./env.sh"; exit 1; }

   build:
     commands: |
       env -u MAKELEVEL $(MAKE) -C hdl/projects/${{ HDL_PROJECT }}/${{ HDL_BOARD }}
     depends_on:
       - sdk-envsetup

   clean:
     commands: |
       $(MAKE) -C hdl/projects/${{ HDL_PROJECT }}/${{ HDL_BOARD }} clean

   gits:
     - name: hdl
       url: https://github.com/analogdevicesinc/hdl.git
       commit: main

The HDL build scripts expect to run as the top-level ``make``: some projects
(those using the transceiver wizard, such as the JESD204 designs) compute
directory names differently when nested inside another ``make``. ``env -u
MAKELEVEL`` starts the HDL build as a top-level ``make``.

Step 3: Add the Environment Script
----------------------------------

Every command in the generated Makefile runs in its own shell, so the manifest
cannot load Vivado's ``settings64.sh`` for later commands. Instead, the target
ships a small script that users source in their shell. Create
``my-manifests/targets/adi-hdl/env.sh``:

.. code-block:: bash

   # Load the Vivado environment. Source this from the workspace root:
   #   VIVADO_DIR=<vivado-install-dir> . ./env.sh
   . "${VIVADO_DIR:?set VIVADO_DIR to your Vivado installation directory}/settings64.sh"

``copy_files`` resolves the relative ``source: env.sh`` against the target
directory and copies the file into the workspace during ``cim init``.

Step 4: Initialize and Build
----------------------------

.. code-block:: bash

   cim init --target adi-hdl --source ./my-manifests --install
   cd ~/dsdk-adi-hdl
   VIVADO_DIR=<vivado-install-dir> . ./env.sh
   make sdk-build

Replace ``<vivado-install-dir>`` with the directory that contains Vivado's
``settings64.sh``. To target a different board without editing the manifest,
set the variables in the environment:

.. code-block:: bash

   HDL_PROJECT=ad9081_fmca_ebz HDL_BOARD=zcu102 make sdk-build

Do not pass them as ``make`` arguments (``make HDL_PROJECT=... sdk-build``).
Make forwards command-line variables to every sub-make, and the HDL build
scripts treat each one as a build parameter: they create a separate project
variant named after it and the build fails.

What You Learned
----------------

- **External vendor tools.** Vivado and Quartus are large tools installed
  separately. The workspace provides a script to load their environment, and
  ``envsetup`` fails early with a clear message when it has not been loaded.
- **Explicit dependencies.** ``depends_on: [sdk-envsetup]`` makes
  ``make sdk-build`` run the check first.
- **copy_files for helper files.** Scripts and other files in the manifest
  repository can be copied into every workspace; remote files can be
  downloaded and cached in the mirror the same way.
- **Variables for project selection.** ``HDL_PROJECT`` and ``HDL_BOARD`` let
  users target different boards at build time without editing the manifest.
