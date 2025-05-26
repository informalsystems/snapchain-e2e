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
      "docker load < /data/snapchain-image.tar",
      "mkdir -p /app/config"
    ]
  }
}

# Create file with data on all infra created.
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
resource "terraform_data" "config-gen" {
  depends_on = [ local_file.infra_data ]
  provisioner "local-exec" {
    command     = "../target/debug/setup_remote_testnet --infra-path ${var.testnet_dir}/infra-data.json"
    working_dir = ".."
  }
}

# Upload config files to corresponding nodes.
resource "terraform_data" "upload-config" {
  triggers_replace = [
    terraform_data.config-gen,
    # digitalocean_droplet.nodes[count.index].id,
    terraform_data.nodes_done,
  ]

  count = local.num_nodes

  connection {
    host        = digitalocean_droplet.nodes[count.index].ipv4_address
    timeout     = var.ssh_timeout
    private_key = tls_private_key.ssh.private_key_openssh
  }

  provisioner "file" {
    source      = "../${var.testnet_dir}/${digitalocean_droplet.nodes[count.index].name}/config.toml"
    destination = "/app/config/config.toml"
  }
}
