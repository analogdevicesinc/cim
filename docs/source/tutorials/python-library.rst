Tutorial: Python Library with C Dependencies
============================================

This tutorial builds
`analogdevicesinc/pyadi-iio <https://github.com/analogdevicesinc/pyadi-iio>`_
and its C dependency `libiio <https://github.com/analogdevicesinc/libiio>`_.
You will learn how to handle mixed C/Python builds, use pip profiles, and
set up install targets with sentinel files.

What You Will Build
-------------------

A manifest that:

- Builds libiio (C library) from source with CMake and installs it into the
  workspace
- Installs pyadi-iio (Python) into the workspace virtual environment
- Provides pip profiles for different use cases
- Runs a smoke test as the test target

Step 1: Create the Manifest Structure
-------------------------------------

.. code-block:: bash

   mkdir -p my-manifests/targets/adi-pyadi-iio

Step 2: Write sdk.yml
---------------------

Create ``my-manifests/targets/adi-pyadi-iio/sdk.yml``:

.. code-block:: yaml

   variables:
     PREFIX: ${{ WORKSPACE }}/.local

   install:
     - name: libiio
       sentinel: true
       depends_on_gits:
         - libiio
       commands:
         - cmake -S libiio -B libiio/build -DCMAKE_INSTALL_PREFIX=$(PREFIX) -DCMAKE_INSTALL_LIBDIR=lib -DINSTALL_UDEV_RULE=OFF -DWITH_IIOD=OFF
         - cmake --build libiio/build
         - cmake --install libiio/build

   build:
     commands: |
       cmake --build libiio/build
       cmake --install libiio/build
     depends_on:
       - install-libiio

   test:
     commands: |
       LD_LIBRARY_PATH=$(PREFIX)/lib $(PREFIX)/bin/iio_info -V
       LD_LIBRARY_PATH=$(PREFIX)/lib .venv/bin/python -c "import adi; print('pyadi-iio', adi.__version__)"

   clean:
     commands: |
       cmake --build libiio/build --target clean

   gits:
     - name: libiio
       url: https://github.com/analogdevicesinc/libiio.git
       commit: v1.0.0

     - name: pyadi-iio
       url: https://github.com/analogdevicesinc/pyadi-iio.git
       commit: main

The ``install`` step configures, builds and installs libiio into
``<workspace>/.local`` once. The IIO daemon (``iiod``) is not needed on the
host and is left out. ``sdk-build`` depends on it, so
``make sdk-build`` works in a fresh workspace and then rebuilds incrementally.
The test commands point ``LD_LIBRARY_PATH`` at the workspace copy of libiio,
since it is not installed system-wide.

Step 3: Add OS Dependencies
---------------------------

Create ``my-manifests/targets/adi-pyadi-iio/os-dependencies.yml``:

.. code-block:: yaml

   pyadi_deps: &pyadi_deps
     - build-essential
     - cmake
     - libusb-1.0-0-dev
     - libxml2-dev
     - libavahi-client-dev
     - libzstd-dev
     - pkg-config
     - python3-venv

   linux:
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
         - gcc
         - make
         - pkgconf-pkg-config
         - libusb1-devel
         - libxml2-devel
         - avahi-devel
         - libzstd-devel

Step 4: Add Python Dependencies
-------------------------------

Python packages, including pyadi-iio itself, belong in
``python-dependencies.yml``. Create
``my-manifests/targets/adi-pyadi-iio/python-dependencies.yml``:

.. code-block:: yaml

   profiles:
     minimal:
       packages: []

     default:
       packages:
         - ./pyadi-iio

     dev:
       packages:
         - ./pyadi-iio
         - pytest
         - pre-commit

     docs:
       packages:
         - ./pyadi-iio
         - sphinx
         - sphinx-rtd-theme
         - myst-parser

   default: default

Packages are installed from the workspace root, so ``./pyadi-iio`` installs
the cloned repository together with its own dependencies (``numpy`` and the
``pylibiio`` bindings).

Step 5: Initialize and Build
----------------------------

.. code-block:: bash

   cim init --target adi-pyadi-iio --source ./my-manifests --full
   cd ~/dsdk-adi-pyadi-iio
   make sdk-test

``--full`` installs the OS packages, installs the default pip profile into
``.venv``, generates the Makefile, and runs the libiio install step. Use
``--install`` instead if the OS packages are already present.

.. note::

   The Python bindings look for libiio through the system library cache
   before ``LD_LIBRARY_PATH``. If an older libiio is installed system-wide
   (for example the ``libiio0`` package, or a 0.x build in ``/usr/local/lib``),
   ``make sdk-test`` fails with ``libiio version 1.x required, found version
   0``. Remove the older library, or use a host or container without it.

What You Learned
----------------

- **Mixed-language dependencies.** libiio (C) is built by an ``install``
  step; the Python bindings find it through ``LD_LIBRARY_PATH``.
- **Install with sentinel.** With ``sentinel: true``, the libiio
  configure/build/install step runs once; CIM skips it while
  ``.cim/libiio-installed`` exists. Run it again with
  ``cim install tools libiio --force``.
- **Pip profiles.** Developers pick the profile that fits their task:
  ``cim install pip --profile dev`` for testing tools,
  ``cim install pip --profile docs`` for documentation builds.
- **Test integration.** The ``test`` section maps to ``make sdk-test``.
