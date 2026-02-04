#!/usr/bin/env python3
import json
import os
import sys


def metrics(service):
    with open(f"docs/reference/{service}-metrics.json") as f:
        metrics = json.load(f)

    os.makedirs(f"docs/reference/metrics/{service}", exist_ok=True)

    def add_period(s):
        if not s.endswith("."):
            return f"{s}."
        return s

    for m in metrics:
        name = m["name"]
        help = add_period(m["help"])
        doc = m["doc"]
        mt = m["metric_type"]
        labels = m["label_names"]
        buckets = m["buckets"]
        pruning = m["pruning"] == "Pruning"

        with open(f"docs/reference/metrics/{service}/{name}.md", "w") as output:
            output.write(f"# {name}\n\n")
            output.write("```\n")
            output.write(f"Type: {mt}\n")

            if len(labels) > 0:
                output.write(f"Labels: {', '.join(labels)}\n")
            if len(buckets) > 0:
                output.write(f"Buckets: {', '.join(str(b) for b in buckets)}\n")

            output.write("```\n")

            output.write(f"{help}\n\n")

            if pruning:
                output.write(
                    "\n!!! note\n    This metric is subject to *pruning*, which means that it may age out and reset to zero when the corresponding internal resources idle- or age-out of the system.  This is a memory management measure to prevent otherwise unbounded growth of memory over time.\n"
                )

            if len(labels) > 0:
                output.write(
                    "\n!!! info\n    This metric has labels which means that the system will track the metric for each combination of the possible labels that are active.  Certain labels, especially those that correlate with source or destination addresses or domains, can have high cardinality.  High cardinality metrics may require some care and attention when provisioning a downstream metrics server.\n"
                )

            if doc:
                output.write(f"{doc}\n\n")

            if len(buckets) > 0:
                output.write("\n## Histogram\n")
                output.write(
                    "This metric is a histogram which means that it is exported as three underlying metrics:\n\n"
                )
                output.write(
                    f"  * `{name}_count` - a counter tracking how many events have been accumulated into the histogram\n"
                )
                output.write(
                    f"  * `{name}_sum` - a counter tracking the total value of all of the events have been accumulated into the histogram\n"
                )
                output.write(
                    f'  * `{name}_bucket` - a counter tracking the number of events that fall within the various buckets shown above.  This counter has an additional `le` label that indicates the bucket threshold.  For example, the first bucket for this histogram will generate a label `le="{buckets[0]}"` which will keep track of the number of events whose value was *less-or-equal* (le) that value.\n'
                )
                output.write(
                    f"\nThe recommended visualization for a histogram is a heatmap based on `{name}_bucket`.\n"
                )

                output.write(
                    f"\nWhile it is possible to calculate a mean average for `{name}` by computing `{name}_sum / {name}_count`, it can be difficult to reason about what that value means if the traffic patterns are not uniform since the launch of the process.  We strongly recommend using a heatmap visualization instead of computing an average value.\n"
                )


metrics("kumod")
