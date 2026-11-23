"""Date helpers."""


def is_leap_year(year):
    """True for a leap year in the Gregorian calendar."""
    raise NotImplementedError


def days_in_month(year, month):
    """The number of days in the month (1 to 12). Any other month raises ValueError."""
    raise NotImplementedError


def days_between(first, second):
    """Days from first to second, two ISO dates "YYYY-MM-DD" (negative when second is earlier). An invalid date raises
    ValueError."""
    raise NotImplementedError
