How to Use Docker for Containerized Development
=================================================

.. note::

   Docker support is experimental. Command options may change in future
   versions.

The ``cim docker create`` command generates Dockerfiles for containerized SDK
development. Since it needs cross-compiled ``cim`` binaries for the target,
this command should be run from the ``cim`` source directory.

Generate a Dockerfile
---------------------

.. code-block:: bash

   cim docker create --target optee-qemu-v8 --distro ubuntu:22.04

Build and Run
-------------

.. code-block:: bash

   docker build -t sdk-dev .
   docker run -it sdk-dev bash

Options
-------

- ``--distro``: Linux distribution (e.g., ``ubuntu:22.04``, ``fedora:42``)
- ``--profile``: Python profile for documentation tools
- ``--force-https``: Convert git URLs to HTTPS (useful for corporate proxies)
- ``--match``: Filter repositories by regex pattern
