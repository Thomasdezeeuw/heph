#!/usr/bin/env bash

# Get the two csv files (permanent and provisional) from:
# https://www.iana.org/assignments/message-headers/message-headers.xhtml
# Remove the header from both file and run:
# $ cat perm-headers.csv prov-headers.csv | ./parse.bash

set -eu

clean_reference_partial() {
	local reference="$1"

	# Remove '[' .. ']'.
	if [[ "${reference:0:1}" == "[" ]]; then
		if [[ "${reference: -1}" == "]" ]]; then
			reference="${reference:1:-1}"
		else
			reference="${reference:1}"
		fi
	fi

	# Wrap links in '<' .. '>'.
	if [[ "$reference" == http* ]]; then
		reference="<$reference>"
	fi

	if [[ "${reference:0:3}" == "RFC" ]]; then
		# Add a space after 'RFC'.
		reference="RFC ${reference:3}"
		# Remove comma and lower case section.
		reference="${reference/, S/ s}"
	fi

	echo -n "$reference"
}

clean_reference() {
	local reference="$1"
	local partial="${2:-false}"

	# Remove '"' .. '"'.
	if [[ "${reference:0:1}" == "\"" ]]; then
		reference="${reference:1:-1}"
	fi

	# Some references are actually multiple references inside '[' .. ']'.
	# Clean them one by one.
	IFS="]" read -ra refs <<< "$reference"
	reference=""
	for ref in "${refs[@]}"; do
		reference+="$(clean_reference_partial "$ref")"
		reference+=', '
	done

	echo "${reference:0:-2}" # Remove last ', '.
}

# Collect all known header name by length in `header_names`.
declare -a header_names
while IFS=$',' read -r name template protocol status reference; do
	# We're only interested in HTTP headers.
	if [[ "http" != "$protocol" ]]; then
		continue
	fi

	reference="$(clean_reference "$reference")"
	const_name="${name^^}"            # To uppercase.
	const_name="${const_name//\-/\_}" # '-' -> '_'.
	const_name="${const_name// /\_}"  # ' ' -> '_'.
	const_value="${name,,}"           # To lowercase.
	value_length="${#const_value}"    # Value length.
	docs="#[doc = \"$name.\\\\n\\\\n$reference.\"]"

	header_names[$value_length]+="$docs|$const_name|$const_value
"
done

# Add non-standard headers.
# X-Request-ID.
header_names[12]+="#[doc = \"X-Request-ID.\"]|X_REQUEST_ID|x-request-id
"

for value_length in "${!header_names[@]}"; do
	values="${header_names[$value_length]}"
	echo "        $value_length: ["
	while IFS=$'|' read -r docs const_name const_value; do
		printf "            $docs\n            ($const_name, \"$const_value\"),\n"
	done <<< "${values:0:-1}" # Remove last new line.
	echo "        ],";
done
