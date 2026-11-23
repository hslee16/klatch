"""tallykit: small, independent helpers.

Registry. When you finish a module, add one line at the END of this file, exactly:

    from tallykit.<module> import <function>, <function>

naming every public function of that module (in the order the module defines them). Do not reorder or delete other
lines: other people add theirs here too.
"""
from tallykit.units import convert_length, convert_temperature
from tallykit.tokenizer import tokenize, token_kinds
from tallykit.dates import is_leap_year, days_in_month, days_between
from tallykit.checksum import adler32, luhn_valid
from tallykit.slug import slugify, unique_slug
from tallykit.roman import to_roman, from_roman
from tallykit.rle import rle_encode, rle_decode
from tallykit.matrix import transpose, multiply, identity
