Tutorial: FPGA HDL Designs
==========================

This tutorial builds
`analogdevicesinc/hdl <https://github.com/analogdevicesinc/hdl>`_
reference designs with Xilinx Vivado. You will learn how to integrate
external vendor tools, set up multi-repo build dependencies, and use
``copy_files`` for artifact management.

What You Will Build
-------------------

A manifest that:

- Sources the Vivado environment at build time (no ``toolchains`` section)
- Builds HDL designs with configurable project and board variables
- Downloads board support packages via ``copy_files``

Step 1: Create the Manifest Structure
--------------------------------------

.. code-block:: bash

   mkdir -p my-manifests/targets/adi-hdl

Step 2: Write sdk.yml
---------------------

Create ``my-manifests/targets/adi-hdl/sdk.yml``:

.. code-block:: yaml

   mirror: $HOME/tmp/mirror

   variables:
     VIVADO: /opt/Xilinx/Vivado/2023.2
     HDL_PROJECT: ad9361
     HDL_BOARD: zed

   envsetup:
     - source ${{ VIVADO }}/settings64.sh

   build:
     - >-
       $(MAKE) -C hdl/projects/${{ HDL_PROJECT }}/${{ HDL_BOARD }}

   clean:
     - >-
       $(MAKE) -C hdl/projects/${{ HDL_PROJECT }}/${{ HDL_BOARD }}
       clean

   copy_files:
     - source: https://wiki.analog.com/resources/fpga/docs/hdl/downloads/ad9361_zed_bsp.zip
       dest: downloads/bsp.zip
       cache: true

   gits:
     - name: hdl
       url: https://github.com/analogdevicesinc/hdl.git
       commit: main
       build:
         - >-
           $(MAKE) -C hdl/projects/${{ HDL_PROJECT }}/${{ HDL_BOARD }}

     - name: linux
       url: https://github.com/analogdevicesinc/linux.git
       commit: main
       build_depends_on:
         - hdl

Step 3: Initialize and Build
-----------------------------

.. code-block:: bash

   cim init --target adi-hdl --source ./my-manifests
   cd ~/dsdk-adi-hdl
   cim makefile
   make sdk-build

To target a different board without editing the manifest:

.. code-block:: bash

   make HDL_PROJECT=ad9081 HDL_BOARD=vcu118 sdk-build

What You Learned
----------------

- **External vendor tools.** Vivado and Quartus are large tools installed
  separately. Use ``envsetup`` to source their environment scripts instead of
  downloading them through CIM.
- **Build dependencies.** ``build_depends_on`` ensures the HDL bitstream is
  built before the Linux kernel image that ships alongside it.
- **copy_files for artifacts.** Board support packages or reference bitstreams
  can be downloaded and cached in the mirror.
- **Variables for project selection.** ``HDL_PROJECT`` and ``HDL_BOARD`` let
  users target different boards at build time without editing the manifest.
