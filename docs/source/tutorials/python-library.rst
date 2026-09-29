Tutorial: Python Library with C Dependencies
=============================================

This tutorial builds
`analogdevicesinc/pyadi-iio <https://github.com/analogdevicesinc/pyadi-iio>`_
and its C dependency `libiio <https://github.com/analogdevicesinc/libiio>`_.
You will learn how to handle mixed C/Python builds, use pip profiles, and
set up install targets with sentinel files.

What You Will Build
-------------------

A manifest that:

- Builds libiio (C library) from source with CMake
- Installs pyadi-iio (Python) with a dependency on libiio
- Provides pip profiles for different use cases
- Runs pytest as the test target

Step 1: Create the Manifest Structure
--------------------------------------

.. code-block:: bash

   mkdir -p my-manifests/targets/adi-pyadi-iio

Step 2: Write sdk.yml
---------------------

Create ``my-manifests/targets/adi-pyadi-iio/sdk.yml``:

.. code-block:: yaml

   mirror: $HOME/tmp/mirror

   build:
     - $(MAKE) -C libiio/build
     - pip install -e pyadi-iio

   test:
     - cd pyadi-iio && pytest -v

   clean:
     - $(MAKE) -C libiio/build clean

   install:
     - name: libiio
       commands: |
         @mkdir -p libiio/build
         @cd libiio/build && cmake .. -DCMAKE_INSTALL_PREFIX=$(pwd)/../../.local
         @$(MAKE) -C libiio/build -j$$(nproc)
         @$(MAKE) -C libiio/build install
       sentinel: .sdk/libiio.installed

   gits:
     - name: libiio
       url: https://github.com/analogdevicesinc/libiio.git
       commit: main
       build:
         - >-
           $(MAKE) -C libiio/build -j$$(nproc)

     - name: pyadi-iio
       url: https://github.com/analogdevicesinc/pyadi-iio.git
       commit: main
       build_depends_on:
         - libiio
       build:
         - pip install -e pyadi-iio

Step 3: Add OS Dependencies
----------------------------

Create ``my-manifests/targets/adi-pyadi-iio/os-dependencies.yml``:

.. code-block:: yaml

   pyadi_deps: &pyadi_deps
     - cmake
     - libusb-1.0-0-dev
     - libxml2-dev
     - libavahi-client-dev
     - python3-dev

   linux-x86_64:
     ubuntu-22.04:
       command: "apt-get install"
       packages: *pyadi_deps
     ubuntu-24.04:
       command: "apt-get install"
       packages: *pyadi_deps
     fedora-42:
       command: "dnf install"
       packages:
         - cmake
         - libusb1-devel
         - libxml2-devel
         - avahi-devel
         - python3-devel

Step 4: Add Python Dependencies
---------------------------------

Create ``my-manifests/targets/adi-pyadi-iio/python-dependencies.yml``:

.. code-block:: yaml

   profiles:
     minimal:
       packages: []

     default:
       packages:
         - numpy
         - pylibiio

     dev:
       packages:
         - numpy
         - pylibiio
         - pytest
         - pre-commit

     docs:
       packages:
         - numpy
         - pylibiio
         - sphinx
         - sphinx-rtd-theme
         - myst-parser

   default: default

Step 5: Initialize and Build
-----------------------------

.. code-block:: bash

   cim init --target adi-pyadi-iio --source ./my-manifests --install
   cd ~/dsdk-adi-pyadi-iio
   cim install tools    # runs libiio cmake/install (once)
   cim makefile
   make sdk-build
   make sdk-test

What You Learned
----------------

- **Mixed-language dependencies.** libiio (C) must be built before pyadi-iio
  (Python) can link against it. ``build_depends_on`` enforces this ordering in
  the generated Makefile.
- **Install with sentinel.** The libiio cmake/install step runs once. On
  subsequent runs CIM skips it if ``.sdk/libiio.installed`` exists.
- **Pip profiles.** Developers pick the profile that fits their task:
  ``cim install pip --profile dev`` for testing tools,
  ``cim install pip --profile docs`` for documentation builds.
- **Test integration.** The ``test`` section maps to ``make sdk-test``, running
  pytest against the pyadi-iio source tree.
