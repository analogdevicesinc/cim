How to Use Private Repositories
===============================

``cim`` uses the host's ``git`` binary for every git operation, so public
repositories need no setup and private ones work with whatever credential
method your git already supports: Git Credential Manager, an SSH agent,
credential helpers, or keychains. ``cim`` does not read tokens itself.

Add ``--verbose`` to ``cim list-targets``, ``cim init`` or ``cim update`` to
trace the git commands it runs:

.. code-block:: bash

   cim list-targets --source https://github.com/org/private-manifests --verbose

GitHub CLI (Recommended for SAML SSO Organizations)
---------------------------------------------------

``gh auth login`` is the recommended setup for private GitHub repositories,
and the only reliable one for organizations that enforce SAML SSO:

.. code-block:: bash

   # Choose: GitHub.com -> HTTPS -> Login with a web browser
   gh auth login

   # For a GitHub Enterprise Server host
   gh auth login --hostname github.your-company.com

When asked, let ``gh`` authenticate git with your GitHub credentials (or run
``gh auth setup-git`` afterwards); this registers ``gh`` as git's credential
helper.

For SAML SSO organizations, also authorize your token for the organization:
open https://github.com/settings/tokens, find the token, and click
**Configure SSO → Authorize**.

SSH
---

SSH keys bypass the HTTPS credential system and work with all GitHub
organizations without any SAML SSO setup. Add your public key at
https://github.com/settings/keys, then map HTTPS URLs to SSH in
``~/.gitconfig``:

.. code-block:: ini

   [url "git@github.com:org/"]
       insteadOf = https://github.com/org/

Manifests can keep their ``https://`` URLs; git rewrites them on the fly.

CI: Access Tokens
-----------------

In headless CI, give git a personal access token with ``repo`` scope (for SAML
SSO organizations, authorize it for the organization as above). With the
GitHub CLI installed, ``gh`` reads the token from ``GH_TOKEN`` (or
``GITHUB_TOKEN``) and serves it to git:

.. code-block:: bash

   export GH_TOKEN=ghp_...
   gh auth setup-git
   cim list-targets --source https://github.com/org/private-manifests

On macOS runners, or any system whose ``/etc/gitconfig`` configures a
credential helper that could prompt interactively, also set
``GIT_CONFIG_NOSYSTEM=1`` so only the token is used.

Authenticated Downloads
-----------------------

Toolchains and ``copy_files`` downloads are fetched by ``cim`` itself, not by
git. Use the ``headers`` or ``basic_auth`` fields for those; see
:doc:`/howto/manage-toolchains` and :doc:`/reference/manifest`.
