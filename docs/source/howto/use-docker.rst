How to Use Docker for Containerized Development
===============================================

.. note::

   Docker support is experimental. Command options may change in future
   versions.

The ``cim docker create`` command generates a Dockerfile that installs the
latest ``cim`` release from GitHub inside the image and runs ``cim init`` for
your target there. It can be run from any directory; no source tree or
cross-compilation is required.

Generate a Dockerfile
---------------------

.. code-block:: bash

   cim docker create --target optee-qemu-v8 \
       --source https://github.com/joabech/cim-manifests \
       --distro ubuntu:24.04

The image runs ``cim init --full --no-sudo --yes --symlink``, so it contains
the cloned repositories, OS packages, toolchains and Python packages. The
workspace is ``/root/dsdk-<target>``.

Apart from ``cim`` itself, the base image only gets ``curl``,
``ca-certificates`` and ``git``. Everything else the workspace needs must be
listed in the target's ``os-dependencies.yml`` for the chosen distribution,
including ``make`` (``build-essential``) and, if the target has Python
dependencies, ``python3-venv`` on Debian and Ubuntu. Otherwise ``cim init``
fails during ``docker build``.

Pass ``--source`` explicitly unless the default manifest source configured on
the build host also works inside the image. The source must be a remote git
URL (``https://``, ``http://`` or ``git@host:...``) that is reachable from
inside the container: a local manifest path is not visible there, and other
schemes such as ``git://`` are treated as local paths.

Build and Run
-------------

.. code-block:: bash

   docker build -t sdk-dev .
   docker run -it sdk-dev

The container starts a shell in the workspace, ready for ``make sdk-build``.
The image looks up the latest ``cim`` release through the unauthenticated
GitHub API, which is rate-limited to 60 requests per hour per IP address. When
the limit is exhausted, ``docker build`` fails in the step that downloads
``cim`` (``curl: (22) ... 403`` followed by a ``404``); wait for the limit to
reset and build again.

Options
-------

- ``--target``, ``-t``: target name (required)
- ``--source``, ``-s``: manifest source URL
- ``--version``, ``-v``: target version (branch or tag of the manifest
  repository)
- ``--distro``, ``-d``: base image (default: ``ubuntu:22.04``); images with
  ``apt-get``, ``dnf``, ``yum`` or ``apk`` are supported
- ``--output``, ``-o``: output path (default: ``Dockerfile``)
- ``--force``, ``-f``: overwrite an existing Dockerfile
