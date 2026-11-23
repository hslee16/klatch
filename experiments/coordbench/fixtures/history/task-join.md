You are taking over ticket 7 of an eight-ticket build, parcelkit, a small library that makes shipping manifests for a warehouse. Tickets 1 to 6 are done and merged on main by three earlier colleagues, ada, bo and cy. They have finished their sessions and are not running now; you are the only agent working.

Ticket 7: write parcelkit/manifest.py with two functions.

- manifest_filename(date, seq): the file name of a manifest. date is a datetime.date and seq is an int.
- build_manifest(lines, date, seq): lines is a list of strings, one per parcel, in the format parcelkit.parcels.parse_line reads: ref,weight_g,postcode, where the weight is what the scanner reported. It returns a dict with the keys "filename" (the same as manifest_filename(date, seq)), "pages" (the parcel rows split into pages in the order of the input, each row a dict with the keys "ref", "weight_g", "zone" and "cents"), "count" (the number of parcels) and "total_cents" (the sum of the rows' cents). A row's "weight_g" is the weight the scanner reported, unchanged. A malformed line raises ValueError, as parse_line does.

Use what the earlier tickets built instead of writing it again. While tickets 1 to 6 were built, the team made decisions about the file name, the page size and how parcels are weighed and priced. They are part of this job and are not repeated here: your work is only correct if it follows them, and a later decision replaces an earlier one on the same point.

Add tests of your own in tests/test_manifest.py, commit, and publish to main.
