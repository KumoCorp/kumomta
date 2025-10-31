# Grafana integration

## Introduction

[Grafana](https://grafana.com/) is a popular tool for visualizing data and generating alerts.

Grafana does not directly integrate with KumoMTA, but is a recommended visualization tool that can consume the feed from Prometheus, which *does* directly integrate with KumomTA.

## Instructions

### Get KumoMTA

 1) Install KumoMTA as per the [../installation/overview.md](installation instructions here)

Before finishing this step, you should ensure that you have correctly set up DNS with a resolving sending domain, MX, PTR, SPF, DKIM, etc.

 2) Ensure that you can inject and deliver mail through KumoMTA.


### Install Prometheus and test

 3) There are detailed instructions [here](https://docs.kumomta.com/userguide/integrations/prometheus/) for installing prometheus in KumoMTA.
  - You can install directly on the server, or in a separate server, or in docker.  We recommend using docker [https://hub.docker.com/r/prom/prometheus](https://hub.docker.com/r/prom/prometheus)
  - Note that `Node Exporter` is not actually required, but will give you access to additional system metrics like free drive space and other OS operational data. 

### Get Grafana
 
 4) Grafana itself can be used in a [number of ways](https://grafana.com/docs/grafana/latest/setup-grafana/installation/).  As long as it can read the Prometheus data feed, it can work for you.
  While you can install directly on the KumoMTA node following the instructions above, we recommend you use a [docker image](https://grafana.com/docs/grafana/latest/setup-grafana/configure-docker/#supported-docker-image-variants) or the [Grafana cloud](https://grafana.com/products/cloud/?plcmt=products-nav) service.

 5) Follow the setup [instructions here](https://docs.kumomta.com/userguide/operation/status/?h=grafana#setting-up-a-grafana-dashboard) to configure the prometheus feed, and get the samepl Gafana dashboard. 


If you have done everything right, you should be able to see your data feed in Grafana within seconds.



