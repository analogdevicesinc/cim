How to Create a Custom Manifest
================================

This guide shows how to create a manifest from scratch for your own project.

Create the Directory Structure
------------------------------

A manifest repository has a ``targets/`` folder with one subdirectory per
target. Each target needs at minimum an ``sdk.yml``.

.. code-block:: bash

   mkdir -p my-manifests/targets/my-sdk

Write a Minimal sdk.yml
------------------------

Create ``my-manifests/targets/my-sdk/sdk.yml``:

.. code-block:: yaml

   mirror: $HOME/tmp/mirror

   gits:
     - name: myproject
       url: https://github.com/myorg/myproject.git
       commit: main
       build:
         - make -C myproject -j$$(nproc)

   build:
     - $(MAKE) -C myproject -j$$(nproc)

   clean:
     - $(MAKE) -C myproject clean

   test:
     - $(MAKE) -C myproject test

Only ``mirror`` is strictly required. Everything else is optional, but a
``gits`` section is needed for anything useful to happen.

Add OS Dependencies (Optional)
-------------------------------

Create ``my-manifests/targets/my-sdk/os-dependencies.yml`` to define system
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

   macos:
     macos-any:
       command: "brew install"
       packages:
         - cmake
         - git

Add Python Dependencies (Optional)
------------------------------------

Create ``my-manifests/targets/my-sdk/python-dependencies.yml``:

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

   cim init --target my-sdk --source ./my-manifests
   cd ~/dsdk-my-sdk
   cim install os-deps --yes
   cim install pip
   cim makefile
   make sdk-build

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
     - $(MAKE) -C myproject ARCH=${{ ARCH }} BUILD_TYPE=${{ BUILD_TYPE }}

Users can then override without editing the manifest:

.. code-block:: bash

   make ARCH=arm64 BUILD_TYPE=debug sdk-build

See :doc:`/reference/manifest` for the complete field reference and
:doc:`/explanation/variables` for how variable expansion works.
