Contributing
============

Contributions are welcome. Please read the
`CONTRIBUTING.md <https://github.com/analogdevicesinc/cim/blob/main/CONTRIBUTING.md>`_
before submitting a pull request.

All commits must be signed off (``git commit -s``) per the
`Developer Certificate of Origin <https://github.com/analogdevicesinc/cim/blob/main/DCO>`_.

Building from Source
--------------------

.. code-block:: bash

   git clone https://github.com/analogdevicesinc/cim.git
   cd cim
   cargo build --release

Running Tests
-------------

.. code-block:: bash

   cargo test

Building Documentation
----------------------

.. code-block:: bash

   cd docs
   pip install -r requirements.txt
   make html

The built documentation will be in ``docs/_build/html/``.
