"""Unit conversion."""

_LENGTH_M = {"mm": 0.001, "cm": 0.01, "m": 1.0, "km": 1000.0, "in": 0.0254, "ft": 0.3048, "yd": 0.9144, "mi": 1609.344}


def convert_length(value, from_unit, to_unit):
    """Convert a length. Units: mm, cm, m, km, in, ft, yd, mi (1 in = 0.0254 m, 1 ft = 0.3048 m, 1 yd = 0.9144 m,
    1 mi = 1609.344 m). Returns a float. An unknown unit raises ValueError."""
    if from_unit not in _LENGTH_M or to_unit not in _LENGTH_M:
        raise ValueError("unknown length unit")
    return float(value) * _LENGTH_M[from_unit] / _LENGTH_M[to_unit]


def convert_temperature(value, from_unit, to_unit):
    """Convert a temperature. Units: C, F, K. Returns a float. An unknown unit raises ValueError."""
    if from_unit not in ("C", "F", "K") or to_unit not in ("C", "F", "K"):
        raise ValueError("unknown temperature unit")
    value = float(value)
    kelvin = value + 273.15 if from_unit == "C" else (value - 32) * 5 / 9 + 273.15 if from_unit == "F" else value
    if to_unit == "C":
        return kelvin - 273.15
    if to_unit == "F":
        return (kelvin - 273.15) * 9 / 5 + 32
    return kelvin
