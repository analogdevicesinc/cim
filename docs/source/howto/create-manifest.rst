How to Create a Custom Manifest
===============================

This guide shows how to create a manifest from scratch for your own project.
The examples use ``https://github.com/myorg/myproject.git`` as a stand-in for
your repository; it is expected to have a Makefile with ``all``, ``clean``
and ``test`` targets.

Create the Directory Structure
------------------------------

A manifest repository has a ``targets/`` folder with one subdirectory per
target. Each target needs at minimum an ``sdk.yml``.

.. code-block:: bash

   mkdir -p my-manifests/targets/my-project

Write a Minimal sdk.yml
-----------------------

Create ``my-manifests/targets/my-project/sdk.yml``:

.. code-block:: yaml

   gits:
     - name: myproject
       url: https://github.com/myorg/myproject.git
       commit: main

   envsetup:
     commands: |
       @command -v cmake >/dev/null || { echo "cmake is required"; exit 1; }

   build:
     commands: |
       $(MAKE) -C myproject
     depends_on:
       - sdk-envsetup

   clean:
     commands: |
       $(MAKE) -C myproject clean

   test:
     commands: |
       $(MAKE) -C myproject test

Each phase (``envsetup``, ``build``, ``test``, ``clean``, ``flash``) takes a
``commands`` block, one shell command per line, and an optional
``depends_on`` list of Makefile targets that must run first. Here
``make sdk-build`` checks for ``cmake`` before building.

No section is strictly required, but a ``gits`` section is needed for anything
useful to happen. A plain list directly under the phase
(``build: ["$(MAKE) -C myproject"]``) is the legacy form; it still works but
cannot express ``depends_on``.

Add OS Dependencies (Optional)
------------------------------

Create ``my-manifests/targets/my-project/os-dependencies.yml`` to define system
packages per distro:

.. code-block:: yaml

   common: &common
     - build-essential
     - git
     - cmake

   linux-x86_64:
     ubuntu-22.04:
       command: "apt-get install"
       packages: *common
     ubuntu-24.04:
       command: "apt-get install"
       packages: *common

   macos:
     macos-any:
       command: "brew install"
       packages:
         - cmake
         - git

Add Python Dependencies (Optional)
----------------------------------

Create ``my-manifests/targets/my-project/python-dependencies.yml``:

.. code-block:: yaml

   profiles:
     default:
       packages:
         - numpy

     dev:
       packages:
         - numpy
         - pytest

   default: default

Test Your Manifest
------------------

.. code-block:: bash

   cim init --target my-project --source ./my-manifests
   cd ~/dsdk-my-project
   cim install os-deps --yes
   cim install toolchains
   cim install pip
   cim makefile
   make sdk-build

``cim install toolchains`` does nothing until you add a ``toolchains``
section (see :doc:`/howto/manage-toolchains`). The same setup in one command:

.. code-block:: bash

   cim init --target my-project --source ./my-manifests --full

``--full`` installs OS packages, toolchains and Python packages, then
generates the Makefile and runs any ``install`` steps. ``--install`` does the
same without the OS packages.

Point ``--source`` at a local directory during development. Once ready,
push the manifest repository to a git remote and use the URL instead.

Add Build Variables
-------------------

Use the ``variables`` section for values that should be overridable at build
time:

.. code-block:: yaml

   variables:
     ARCH: x86_64
     BUILD_TYPE: release

   build:
     commands: |
       $(MAKE) -C myproject ARCH=${{ ARCH }} BUILD_TYPE=${{ BUILD_TYPE }}

Users can then override without editing the manifest:

.. code-block:: bash

   make ARCH=arm64 BUILD_TYPE=debug sdk-build

See :doc:`/reference/manifest` for the complete field reference and
:doc:`/explanation/variables` for how variable expansion works.
