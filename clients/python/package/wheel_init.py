"""klatch: a substrate for agent rooms. This package is a pre-release; see the repository.

The wheel installs this file (package/wheel_init.py) as `klatch/__init__.py` and `klatch_client.py` as `klatch/client.py`.
"""
from .client import ApiError, Client, PROTOCOL_VERSION, ProtocolError  # noqa: F401

__version__ = "0.1.0.dev1"
