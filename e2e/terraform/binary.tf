# While the droplets are being created, export the Docker image.
# The image should already exists with tag snapchain-node
resource "terraform_data" "app-image" {
  provisioner "local-exec" {
    command     = "docker image save snapchain-node -o snapchain-image.tar"
    working_dir = ".."
  }
}

# Upload compressed binary to CC.
resource "terraform_data" "binary-remote" {
  triggers_replace = [
    terraform_data.app-image,
    digitalocean_droplet.cc.id
  ]

  connection {
    host        = digitalocean_droplet.cc.ipv4_address
    timeout     = var.ssh_timeout
    private_key = tls_private_key.ssh.private_key_openssh
  }

  provisioner "file" {
    source      = "../snapchain-image.tar"
    destination = "/root/snapchain-image.tar"
  }
}

# Once cloud-init is done on CC, set up NFS directory and load image to Docker.
resource "terraform_data" "cc-nfs" {
  triggers_replace = [
    digitalocean_droplet.cc.id,
    # terraform_data.cc-done.id,
    terraform_data.binary-remote
  ]

  connection {
    host        = digitalocean_droplet.cc.ipv4_address
    timeout     = var.ssh_timeout
    private_key = tls_private_key.ssh.private_key_openssh
  }

  provisioner "remote-exec" {
    inline = [
      # Block until cloud-init completes.
      "cloud-init status --wait > /dev/null 2>&1",
      # Set up NFS.
      "mkdir /data",
      "chown nobody:nogroup /data",
      "systemctl start nfs-kernel-server",
      "systemctl enable nfs-kernel-server",
      # Now that the NFS directory is ready, put the file in it.
      "mv /root/snapchain-image.tar /data",
      "chown nobody:nogroup /data/snapchain-image.tar",
    ]
  }
}