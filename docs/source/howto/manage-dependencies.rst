How to Manage Dependencies
==========================

This guide covers managing OS packages and Python dependencies across
different platforms and use cases.

OS Dependencies
---------------

Create ``os-dependencies.yml`` alongside your ``sdk.yml``.

Define packages per distribution and architecture:

.. code-block:: yaml

   common_deps: &common
     - build-essential
     - git
     - cmake

   linux-x86_64:
     ubuntu-22.04:
       command: "apt-get install"
       packages: *common
     fedora-42:
       command: "dnf install"
       packages:
         - gcc
         - gcc-c++
         - git
         - cmake

   macos:
     macos-any:
       command: "brew install"
       packages:
         - cmake
         - git

Use YAML anchors (``&name`` / ``*name``) to share package lists across
distributions that use the same names.

Install with:

.. code-block:: bash

   cim install os-deps          # interactive (asks for sudo)
   cim install os-deps --yes    # non-interactive
   cim install os-deps --no-sudo  # skip sudo (useful for CI with root)

CIM detects the host OS and distribution automatically and runs the
corresponding command.

Python Dependencies
-------------------

Create ``python-dependencies.yml`` alongside your ``sdk.yml``.

Organize packages into profiles for different use cases:

.. code-block:: yaml

   profiles:
     minimal:
       packages: []

     default:
       packages:
         - numpy

     dev:
       packages:
         - numpy
         - pytest
         - pre-commit

     docs:
       packages:
         - numpy
         - sphinx
         - myst-parser

   default: default

Install with:

.. code-block:: bash

   cim install pip                    # uses the "default" profile
   cim install pip --profile dev      # specific profile
   cim install pip --profile dev,docs # multiple profiles

Packages are installed into the workspace ``.venv`` directory.

Use ``--force`` to reinstall even if the virtual environment already
exists. Use ``--symlink`` to symlink the venv from the mirror.

Best Practices
--------------

- Provide at least ``minimal``, ``default``, and ``dev`` profiles so users
  install only what they need.
- Use YAML anchors in ``os-dependencies.yml`` to avoid duplicating package
  lists across similar distributions.
- Test on multiple distributions. Package names differ between apt and dnf.
