CLI Reference
=============

list-targets
------------

Show available SDK targets from a manifest repository.

.. code-block:: bash

   cim list-targets [--source URL|PATH] [--target NAME]

init
----

Initialize workspace from a target.

.. code-block:: bash

   cim init --target NAME [--workspace PATH] [--version VERSION]
             [--match REGEX] [--install] [--full] [--symlink] [--no-mirror]

.. list-table::
   :header-rows: 1

   * - Option
     - Description
   * - ``--target``, ``-t``
     - Target name from the manifest repository
   * - ``--workspace``, ``-w``
     - Workspace directory (default: ``$HOME/dsdk-<target>``)
   * - ``--version``, ``-v``
     - Target version
   * - ``--match``, ``-m``
     - Only clone repos matching pattern (supports comma-separated values)
   * - ``--install``
     - Install toolchains and pip packages after init
   * - ``--full``
     - Complete setup including OS dependencies (requires sudo)
   * - ``--symlink``
     - Install to mirror with symlinks in workspace
   * - ``--no-mirror``
     - Disable mirroring for this workspace

update
------

Update git repos in workspace to the commits specified in sdk.yml.

.. code-block:: bash

   cim update [--match REGEX] [--no-mirror]

makefile
--------

Generate Makefile from sdk.yml. Must be run from within a workspace.

.. code-block:: bash

   cim makefile [--no-dividers]

foreach
-------

Run a command in each repository in the workspace.

.. code-block:: bash

   cim foreach "COMMAND" [--match REGEX]

Example:

.. code-block:: bash

   cim foreach "git status"
   cim foreach "git log --oneline -5" --match "linux,hdl"

add
---

Add a git repository to the workspace sdk.yml.

.. code-block:: bash

   cim add --name NAME --url URL --commit COMMIT

install
-------

os-deps
~~~~~~~

Install system packages from os-dependencies.yml. CIM detects the host OS
and distribution automatically.

.. code-block:: bash

   cim install os-deps [--yes] [--no-sudo]

pip
~~~

Install Python packages from python-dependencies.yml into the workspace
``.venv``.

.. code-block:: bash

   cim install pip [--profile PROFILE] [--symlink] [--force]

Example:

.. code-block:: bash

   cim install pip --profile dev,docs

toolchains
~~~~~~~~~~

Download and extract toolchains defined in sdk.yml.

.. code-block:: bash

   cim install toolchains [--symlink] [--force]

tools
~~~~~

Run install targets defined in the ``install`` section of sdk.yml.

.. code-block:: bash

   cim install tools [NAME] [--all] [--list]

docs
----

create
~~~~~~

Aggregate documentation from repositories in the workspace.

.. code-block:: bash

   cim docs create [--force] [--theme THEME] [--symlink]

build
~~~~~

Build the aggregated documentation.

.. code-block:: bash

   cim docs build [--format html|pdf|epub]

serve
~~~~~

Serve documentation locally with a development server.

.. code-block:: bash

   cim docs serve [--port PORT]

release
-------

Create release tags across workspace repositories.

.. code-block:: bash

   cim release --tag TAG [--include PATTERNS] [--exclude PATTERNS] [--dry-run]

config
------

Manage user configuration file.

.. code-block:: bash

   cim config [--list] [--get KEY] [--create] [--edit] [--validate]

See :doc:`/reference/configuration` for configuration file details.

utils
-----

hash-copy-files
~~~~~~~~~~~~~~~

Compute and update SHA256 hashes for ``copy_files`` entries in sdk.yml.

.. code-block:: bash

   cim utils hash-copy-files [--dry-run] [--verbose] [--add-missing]

hash-toolchains
~~~~~~~~~~~~~~~

Compute and update SHA256 hashes for toolchain archives in the local mirror.

.. code-block:: bash

   cim utils hash-toolchains [--dry-run] [--verbose] [--add-missing]

sync-copy-files
~~~~~~~~~~~~~~~

Re-run ``copy_files`` to sync files to workspace.

.. code-block:: bash

   cim utils sync-copy-files [--dry-run] [--verbose] [--force]

update
~~~~~~

Self-update the ``cim`` binary to the latest release.

.. code-block:: bash

   cim utils update

docker
------

Generate Dockerfiles for containerized SDK development (experimental).

.. code-block:: bash

   cim docker create --target NAME --distro DISTRO

.. list-table::
   :header-rows: 1

   * - Option
     - Description
   * - ``--distro``
     - Linux distribution (e.g., ``ubuntu:22.04``, ``fedora:42``)
   * - ``--profile``
     - Python profile for documentation tools
   * - ``--force-https``
     - Convert git URLs to HTTPS
   * - ``--match``
     - Filter repositories by regex pattern
