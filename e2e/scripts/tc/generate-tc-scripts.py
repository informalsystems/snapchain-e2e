#!/usr/bin/python3
import sys
import csv
import subprocess
import json


def read_matrix(csv_file):
    with open(csv_file) as f:
        reader = csv.reader(f)
        header = next(reader)[1:]
        matrix = {}
        for row in reader:
            row_zone = row[0]
            matrix[row_zone] = {}
            for i, col_zone in enumerate(header):
                matrix[row_zone][col_zone] = int(row[i + 1])
    return header, matrix


def read_infra_data(infra_data_file):
    with open(infra_data_file) as f:
        infra_data = json.load(f)
        data = {}
        ips = []
        for instance, instance_data in infra_data["instances"].items():
            data[instance] = instance_data["private_ip"]
            ips.append(instance_data["private_ip"])
        return data, ips


def execute_command(cmd):
    subprocess.run(cmd, shell=True, check=True)


def build_tc_commands(header, matrix, ips, local_ip, bandwidth="1gbit"):
    commands = []
    num_zones = len(header)
    local_index = ips.index(local_ip)
    local_zone = header[local_index % num_zones]

    commands.append("tc qdisc del dev eth1 root 2> /dev/null || true")
    commands.append("tc qdisc add dev eth1 root handle 1: htb default 10")
    commands.append(f"tc class add dev eth1 parent 1: classid 1:1 htb rate {bandwidth}")
    commands.append(
        f"tc class add dev eth1 parent 1:1 classid 1:10 htb rate {bandwidth}"
    )
    commands.append("tc qdisc add dev eth1 parent 1:10 handle 10: sfq perturb 10")

    handle = 11
    for zone in header:
        zone_machines = []
        for ip in ips:
            if ip != local_ip:
                idx = ips.index(ip)
                z = header[idx % num_zones]
                if z == zone:
                    zone_machines.append(ip)

        if not zone_machines:
            continue

        latency = matrix[local_zone][zone]
        if latency > 0:
            delta = latency // 20
            if delta == 0:
                delta = 1
            commands.append(
                f"tc class add dev eth1 parent 1:1 classid 1:{handle} htb rate {bandwidth}"
            )
            commands.append(
                f"tc qdisc add dev eth1 parent 1:{handle} handle {handle}: netem delay {latency}ms {delta}ms distribution normal"
            )
            for ip in zone_machines:
                commands.append(
                    f"tc filter add dev eth1 protocol ip parent 1: prio 1 u32 match ip dst {ip}/32 flowid 1:{handle}"
                )
            handle += 1
    return commands


def main():
    csv_file = sys.argv[1]
    infra_data_file = sys.argv[2]
    bandwidth = sys.argv[3] if len(sys.argv) > 3 else "1gbit"

    header, matrix = read_matrix(csv_file)
    infra_data, ips = read_infra_data(infra_data_file)

    for instance, ip in infra_data.items():
        commands = build_tc_commands(header, matrix, ips, ip)
        with open(f"nodes/{instance}/tc-setup.sh", "w") as f:
            f.write("#!/bin/bash\n")
            for cmd in commands:
                f.write(f"{cmd}\n")
        execute_command(f"chmod +x nodes/{instance}/tc-setup.sh")


if __name__ == "__main__":
    main()
