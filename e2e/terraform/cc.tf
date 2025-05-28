resource "tls_private_key" "ssh" {
  algorithm = "ED25519"
}

resource "digitalocean_ssh_key" "cc" {
  name       = lower("autossh-project-${var.project_name}")
  public_key = tls_private_key.ssh.public_key_openssh
}

resource "digitalocean_droplet" "cc" {
  # depends_on = [digitalocean_vpc.testnet-vpc]
  name     = "cc"
  image    = "debian-12-x64"
  region   = var.region
  tags     = concat(var.tags, ["cc", var.region])
  size     = var.cc_size
  ssh_keys = concat(var.ssh_keys, [digitalocean_ssh_key.cc.id])
  vpc_uuid = digitalocean_vpc.testnet-vpc.id
  user_data = templatefile("user-data/cc-data.yaml", {
    grafana_datasource_prometheus = filebase64("../monitoring/grafana/provisioning/datasources/prometheus.yml")
    grafana_datasource_graphite   = filebase64("../monitoring/grafana/provisioning/datasources/graphite.yml")
    grafana_dashboards_config     = filebase64("../monitoring/grafana/provisioning/dashboards/dashboards.yaml")
  })
  connection {
    host        = digitalocean_droplet.cc.ipv4_address
    timeout     = var.ssh_timeout
    private_key = tls_private_key.ssh.private_key_openssh
  }
  provisioner "file" {
    source      = "../monitoring/grafana/provisioning/dashboards-data"
    destination = "/root"
  }
  provisioner "file" {
    content     = tls_private_key.ssh.private_key_openssh
    destination = "/root/.ssh/id_rsa"
  }
}

# # Once cloud-init is done on CC, set up and start a DNS server.
# resource "terraform_data" "cc-dns" {
#   triggers_replace = [
#     digitalocean_droplet.nodes,
#     digitalocean_droplet.cc.id
#   ]

#   connection {
#     host        = digitalocean_droplet.cc.ipv4_address
#     timeout     = var.ssh_timeout
#     private_key = tls_private_key.ssh.private_key_openssh
#   }

#   provisioner "file" {
#     content     = templatefile("templates/hosts.tmpl", { nodes = local.nodes, cc = local.cc })
#     destination = "/etc/hosts"
#   }

#   provisioner "remote-exec" {
#     inline = [
#       # "cloud-init status --wait  > /dev/null 2>&1",
#       "while [ ! -f /etc/done ]; do sleep 1; done",
#       "systemctl reload-or-restart dnsmasq"
#     ]
#   }
# }

resource "local_file" "cc-ip" {
  depends_on = [digitalocean_droplet.cc]
  content  = local.cc.ip
  filename = "../${var.testnet_dir}/.cc-ip"
}

# Generate Prometheus config and start the monitoring services.
resource "terraform_data" "prometheus-config" {
  triggers_replace = [
    digitalocean_droplet.cc.id
  ]

  connection {
    host        = digitalocean_droplet.cc.ipv4_address
    timeout     = var.ssh_timeout
    private_key = tls_private_key.ssh.private_key_openssh
  }

  provisioner "file" {
    content     = templatefile("templates/prometheus-yml.tmpl", { nodes = local.nodes })
    destination = "/root/docker/prometheus.yml"
  }

  provisioner "remote-exec" {
    inline = [
      "while [ ! -f /root/docker/compose.yml ]; do sleep 1; done",
      "docker compose -f /root/docker/compose.yml --profile monitoring --progress quiet up -d"
    ]
  }
}
