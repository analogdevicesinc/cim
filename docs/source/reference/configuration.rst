Configuration Reference
=======================

CIM uses a TOML configuration file for user-level settings that apply across
all workspaces.

File Location
-------------

- **Unix/Linux/macOS:** ``~/.config/cim/config.toml``
- **Windows:** ``%LOCALAPPDATA%\cim\config.toml``

Create it with:

.. code-block:: bash

   cim config --create

Settings
--------

.. list-table::
   :header-rows: 1

   * - Key
     - Default
     - Description
   * - ``default_source``
     - (none)
     - Default manifest source URL or path, used when ``--source`` is not specified.
   * - ``mirror_path``
     - ``$HOME/tmp/mirror``
     - Override the mirror location for all workspaces.
   * - ``workspace_prefix``
     - ``dsdk-``
     - Prefix for workspace directory names.
   * - ``documentation_dirs``
     - (none)
     - Comma-separated list of additional documentation directories to search.
   * - ``cert_validation``
     - ``strict``
     - TLS certificate validation mode: ``strict``, ``relaxed``, or ``auto``.

Example
-------

.. code-block:: toml

   default_source = "https://github.com/myorg/cim-manifests"
   mirror_path = "/custom/mirror"
   workspace_prefix = "sdk-"
   documentation_dirs = "wiki, manual, reference"
   cert_validation = "strict"

Managing Configuration
----------------------

.. code-block:: bash

   cim config --create     # Create config file with defaults
   cim config --list       # Show current settings
   cim config --get KEY    # Get a specific setting
   cim config --edit       # Open config in editor
   cim config --validate   # Validate config file

Certificate Validation
----------------------

CIM validates TLS certificates by default using strict checking.

.. list-table::
   :header-rows: 1

   * - Mode
     - Behavior
   * - ``strict``
     - Full certificate validation (default).
   * - ``relaxed``
     - Disables certificate validation. **Insecure** — vulnerable to MITM attacks.
   * - ``auto``
     - Tries strict first, falls back to relaxed with a warning.

Override per-command:

.. code-block:: bash

   cim install toolchains --cert-validation=relaxed

.. warning::

   Use ``relaxed`` mode only in trusted networks.
