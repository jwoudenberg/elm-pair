#!/usr/bin/env python

# This script generates a news item on the site for each entry of the changelog.

import re
import sys
import logging

CHANGELOG_HEADER = r'^##\s+(\d{4}-\d{2}-\d{2}):\s+Release (\d+)$'


def main():
    logger = logging.getLogger()
    # As we go through the CHANGELOG line by line, the 'out' variable will
    # point to a (new) markdown file in the content directory for the changelog
    # item we're currently scanning. We start out pointing this to /dev/null,
    # because we don't care about any content in the changelog before the first
    # changelog item.
    out = open('/dev/null', 'w')
    with open('../CHANGELOG.md') as file:
        for line in file:
            if line.startswith('##'): # Recognize markdown header
                match = re.match(CHANGELOG_HEADER, line)
                if not match:
                    logger.error("changelog header with unexpected format")
                    return 1
                date = match.group(1)
                version = match.group(2)
                slug = f'{date}-release-{version}'
                weight = 100000 - int(version)
                out.close()
                out = open(f'content/news/from-changelog-{date}-{slug}.md', 'w+')
                out.write(
                    f"""+++
title = "Release {version}"
weight = {weight}
date = "{date}"
+++""")
            else:
                out.write(line)
    out.close()
    return 0


if __name__ == '__main__':
    sys.exit(main())
