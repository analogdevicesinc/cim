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
     ubuntu-24.04:
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

CIM picks the section for the host:

- The top-level key is ``linux-<arch>`` (``linux-x86_64``, ``linux-aarch64``),
  falling back to ``linux``; ``macos`` and ``windows`` for the other hosts.
- The distribution key is ``<ID>-<VERSION_ID>`` from ``/etc/os-release``
  (e.g. ``ubuntu-24.04``, ``fedora-42``), falling back to the bare ``<ID>``.
  On macOS it is ``macos-any``.

Use YAML anchors (``&name`` / ``*name``) to share package lists across
distributions that use the same names. A ``packages`` entry may also be a list
of lists, which CIM flattens and de-duplicates; this lets you combine a shared
base with extras:

.. code-block:: yaml

   base: &base
     - build-essential
     - git

   x86_extras: &x86_extras
     - gcc-multilib

   linux-x86_64:
     ubuntu-24.04:
       command: "apt-get install"
       packages:
         - *base
         - *x86_extras

Install with:

.. code-block:: bash

   cim install os-deps            # asks for confirmation, runs with sudo
   cim install os-deps --yes      # non-interactive
   cim install os-deps --no-sudo  # skip sudo (useful for CI running as root)

CIM detects the host OS and distribution automatically and runs the
corresponding command with the packages appended. ``--yes`` also passes
``-y`` to ``apt-get``/``dnf`` (``--noconfirm`` to ``pacman``).

Python Dependencies
-------------------

There are two places to declare Python packages:

- **Workspace-wide tools** (docs, linters, test runners, your own Python
  packages) go into ``python-dependencies.yml`` profiles and are installed into
  the shared ``<workspace>/.venv``.
- **A repository's own requirements** go into the ``python-deps`` field of that
  repository's ``gits`` entry. Each such repository gets an isolated
  ``.cim/<name>/.venv``.

python-dependencies.yml
~~~~~~~~~~~~~~~~~~~~~~~

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
   cim install pip --list-profiles    # show the profiles

Packages are passed to pip as they are, from the workspace root, so a profile
can pin versions (``sphinx==7.2.6``) or install a cloned repository
(``./myrepo``). A profile can also point at existing requirements files,
relative to the workspace root:

.. code-block:: yaml

   profiles:
     docs:
       packages:
         - sphinx
       requirements:
         - myrepo/docs/requirements.txt

   default: docs

When ``default`` is omitted, the ``docs`` profile is the default.

Per-repository python-deps
~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: yaml

   gits:
     - name: zephyr
       url: https://github.com/zephyrproject-rtos/zephyr.git
       commit: main
       python-deps: scripts/requirements-base.txt

The path is relative to the repository checkout. ``cim install pip`` creates
``.cim/zephyr/.venv`` and installs the requirements into it, and the generated
Makefile exposes it as ``$(ZEPHYR_VENV)``, so build commands can run
``$(ZEPHYR_VENV)/bin/python``.

Install Options
~~~~~~~~~~~~~~~

Use ``--force`` to recreate the virtual environment. Use ``--symlink`` to keep
the venv in the mirror (``<mirror>/.venv``) and symlink it into the workspace,
so several workspaces share one environment.

If `uv <https://docs.astral.sh/uv/>`_ is on ``PATH``, CIM uses it as a faster
backend; otherwise it uses ``python3 -m venv`` and ``pip``. Either way the
result is a standard virtual environment.

If a shared ``--symlink`` venv breaks (for example after a system Python
upgrade), repair it instead of forcing a reinstall from one workspace:

.. code-block:: bash

   cim install pip --repair

Best Practices
--------------

- Provide at least ``minimal``, ``default``, and ``dev`` profiles so users
  install only what they need.
- Use YAML anchors in ``os-dependencies.yml`` to avoid duplicating package
  lists across similar distributions.
- Test on multiple distributions. Package names differ between apt and dnf.
