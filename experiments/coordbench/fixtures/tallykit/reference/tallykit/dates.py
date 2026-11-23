"""Date helpers."""
import datetime


def is_leap_year(year):
    """True for a leap year in the Gregorian calendar."""
    return year % 4 == 0 and (year % 100 != 0 or year % 400 == 0)


def days_in_month(year, month):
    """The number of days in the month (1 to 12). Any other month raises ValueError."""
    if not isinstance(month, int) or not 1 <= month <= 12:
        raise ValueError("month must be 1 to 12")
    if month == 2:
        return 29 if is_leap_year(year) else 28
    return 30 if month in (4, 6, 9, 11) else 31


def days_between(first, second):
    """Days from first to second, two ISO dates "YYYY-MM-DD" (negative when second is earlier). An invalid date raises
    ValueError."""
    return (datetime.date.fromisoformat(second) - datetime.date.fromisoformat(first)).days
