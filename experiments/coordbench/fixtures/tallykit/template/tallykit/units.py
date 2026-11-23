"""Unit conversion."""


def convert_length(value, from_unit, to_unit):
    """Convert a length. Units: mm, cm, m, km, in, ft, yd, mi (1 in = 0.0254 m, 1 ft = 0.3048 m, 1 yd = 0.9144 m,
    1 mi = 1609.344 m). Returns a float. An unknown unit raises ValueError."""
    raise NotImplementedError


def convert_temperature(value, from_unit, to_unit):
    """Convert a temperature. Units: C, F, K. Returns a float. An unknown unit raises ValueError."""
    raise NotImplementedError
