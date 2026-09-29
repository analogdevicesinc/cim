Tutorial: FPGA HDL Designs
==========================

This tutorial builds
`analogdevicesinc/hdl <https://github.com/analogdevicesinc/hdl>`_
reference designs with vendor EDA tools (Xilinx Vivado, Intel Quartus, and Lattice Radiant).
You will learn how to integrate external vendor tools, explore supported project/carrier board
combinations, and use the guided build flow.

What You Will Build
-------------------

A manifest and workflow that:

- Integrates external vendor tools (Vivado 2025.1, Quartus Prime, Lattice Radiant) with automatic environment detection
- Supports 85+ HDL projects and 160+ carrier board combinations
- Provides an interactive guided wizard (``make guide`` or ``./scripts/build-hdl.sh``)
- Exposes make targets for matrix inspection (``make list-combos``, ``make list-boards``, ``make check-tools``)
- Downloads and generates boot binaries (BOOT.BIN) for supported Zynq / ZynqMP targets

Supported EDA Tools Matrix
--------------------------

Analog Devices HDL reference designs target multiple FPGA carriers, each requiring a specific vendor tool:

.. list-table::
   :header-rows: 1
   :widths: 20 25 35 20

   * - Vendor Tool
     - Recommended Version
     - Carrier Boards
     - Default Environment
   * - **Xilinx Vivado**
     - 2025.1 (for ``hdl_2026_r1``)
     - ``zed``, ``zc702``, ``zc706``, ``zcu102``, ``coraz7s``, ``k26``, ``kv260``, ``vck190``, ``vmk180``, ``vpk180``, ``kcu105``, ``vcu118``, ``ac701``, etc.
     - ``/opt/Xilinx/2025.1/Vivado/settings64.sh`` (or ``$VIVADO``)
   * - **Intel Quartus Prime**
     - Pro 23.4 / Standard
     - ``de10nano``, ``c5soc``, ``a10soc``, ``a10gx``, ``s10soc``, ``fm87``
     - ``/opt/intelFPGA_pro/23.4/quartus`` (or ``$QUARTUS`` / ``$QUARTUS_ROOTDIR``)
   * - **Lattice Radiant / Propel**
     - Radiant 2023+
     - ``lfcpnx`` (Certus-NX)
     - ``/usr/local/radiant`` (or ``$LATTICE_RADIANT``)

Guided Build Workflow
---------------------

Step 1: Initialize the Target
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

.. code-block:: bash

   cim init --target hdl_2026_r1 --source <path-to-manifests> --workspace ~/dsdk-hdl
   cd ~/dsdk-hdl
   cim makefile

Step 2: Inspect Boards and Tool Requirements
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

You can inspect the combinations and tool availability before building:

.. code-block:: bash

   # Check if required EDA tools are installed and found in your environment
   make check-tools

   # List all available projects and their supported carrier boards
   make list-combos

   # List carrier boards for a specific project
   make list-boards HDL_PROJECT=ad353xr

   # View tool matrix summary
   make list-tools

Step 3: Build Using the Guided Wizard
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

Run the interactive guided builder:

.. code-block:: bash

   make guide

Or run the script directly:

.. code-block:: bash

   ./scripts/build-hdl.sh

The guided wizard walks you through:

1. Searching or selecting an **HDL Project** (e.g., ``fmcomms2``, ``adrv9009``, ``ad353xr``, ``cn0561``)
2. Selecting a supported **Carrier Board** for that project
3. Verifying the required **EDA Tool** (Vivado, Quartus, Radiant) in your environment
4. Configuring **Parallel Jobs** (``-j``) and **BOOT.BIN** generation
5. Confirming and executing the build

Step 4: Non-Interactive Builds via Make Overrides
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

You can also trigger builds directly with variables:

.. code-block:: bash

   # Build fmcomms2 for ZedBoard (Vivado) and generate BOOT.BIN
   make HDL_PROJECT=fmcomms2 HDL_BOARD=zed BUILD_BOOT_BIN=true sdk-build

   # Build cn0561 for DE10-Nano (Intel Quartus)
   make HDL_PROJECT=cn0561 HDL_BOARD=de10nano sdk-build

   # Build ad738x_fmc for Lattice Certus-NX (Lattice Radiant)
   make HDL_PROJECT=ad738x_fmc HDL_BOARD=lfcpnx sdk-build

What You Learned
----------------

- **Multi-vendor EDA tool management:** Smart environment detection routes Xilinx designs to Vivado, Intel designs to Quartus, and Lattice designs to Radiant.
- **Project/board matrix discovery:** Dynamically inspect 160+ reference design combinations without manually parsing Makefiles.
- **Interactive vs scriptable build flow:** Choose between the guided wizard (``make guide``) and automated Make variables (``make HDL_PROJECT=... HDL_BOARD=... sdk-build``).

