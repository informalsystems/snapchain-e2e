resource "digitalocean_droplet" "nodes" {
  count     = local.num_nodes
  name      = local.node_names[count.index]
  image     = "debian-12-x64"
  region    = var.region
  tags      = concat(var.tags, [var.region])
  size      = var.small
  vpc_uuid  = digitalocean_vpc.testnet-vpc.id
  ssh_keys  = concat(var.ssh_keys, [digitalocean_ssh_key.cc.id])
  user_data = templatefile("user-data/nodes-data.yaml", {
    id = local.node_names[count.index]
    cc = {
      name        = digitalocean_droplet.cc.name
      ip          = digitalocean_droplet.cc.ipv4_address
      internal_ip = digitalocean_droplet.cc.ipv4_address_private
    }
  })
}

# After a node is created and cloud-init is done, mount the NFS directory and
# and load the Docker image.
resource "terraform_data" "nodes_done" {
  triggers_replace = [
    digitalocean_droplet.nodes[count.index].id,
    terraform_data.cc-nfs.id,
  ]

  count = local.num_nodes

  connection {
    host        = digitalocean_droplet.nodes[count.index].ipv4_address
    timeout     = var.ssh_timeout
    private_key = tls_private_key.ssh.private_key_openssh
  }

  provisioner "remote-exec" {
    inline = [
      "cloud-init status --wait > /dev/null 2>&1",
      "mount /data",
      "docker load < /data/snapchain-image.tar"
    ]
  }
}

# Create file with data once all nodes are created.
resource "local_file" "infra_data" {
  depends_on = [
    digitalocean_droplet.cc,
    digitalocean_droplet.nodes,
  ]
  content = templatefile("${path.module}/templates/infra-data-json.tmpl", {
    subnet         = var.vpc_subnet,
    nodes          = local.nodes,
    cc             = local.cc,
    num_validators = local.num_validators,
    num_full_nodes = local.num_full_nodes,
  })
  filename = "../${var.testnet_dir}/infra-data.json"
}

# Generate config files using infra data.
resource "terraform_data" "config_gen" {
  depends_on = [ local_file.infra_data ]
  provisioner "local-exec" {
    command     = <<-EOT
      cargo build --bin setup_remote_testnet
      ../target/debug/setup_remote_testnet --infra-path ${var.testnet_dir}/infra-data.json --num-shards=${var.num_shards} --first-full-nodes=${var.first_full_nodes}
      ./scripts/tc/generate-tc-scripts.py scripts/tc/latencies.csv nodes/infra-data.json ${var.bandwidth}
    EOT
    working_dir = ".."
  }
}

# Upload config files to corresponding nodes and set up latency emulation.
resource "terraform_data" "upload_config" {
  triggers_replace = [
    terraform_data.config_gen,
    terraform_data.nodes_done,
  ]

  count = local.num_nodes

  connection {
    host        = digitalocean_droplet.nodes[count.index].ipv4_address
    timeout     = var.ssh_timeout
    private_key = tls_private_key.ssh.private_key_openssh
  }

  provisioner "file" {
    source      = "../${var.testnet_dir}/${digitalocean_droplet.nodes[count.index].name}/"
    destination = "/app/config"
  }

  provisioner "remote-exec" {
    inline = [
      "while [ ! -f /app/config/tc-setup.sh ]; do sleep 1; done",
      "chmod +x /app/config/tc-setup.sh",
      "/app/config/tc-setup.sh"
    ]
  }
}
