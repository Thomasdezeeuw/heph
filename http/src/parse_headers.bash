#!/usr/bin/env bash

# Get the csv file with registered headers from
# <https://www.iana.org/assignments/http-fields/http-fields.xhtml>.
# Remove the header from the file and run:
# $ cat field-names.csv | ./src/parse_headers.bash

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
		# Remove double space
		# Lowercase "Section"
		# Remove ": $section_name" part.
		reference=$(echo "$reference" | sed -e 's/Section/section/g' -e 's/:.*//' -e 's/  / /g')
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
while IFS=$',' read -r -a values; do
	# The reference column may contain commas, so we have to use an array to
	# extract the columns we're interested in. Specifically in the case of
	# the reference column we need to join multiple columns.

	name="${values[0]}"

	# Only include "permanent" headers, ignoring deprecated and obsoleted
	# headers.
	status="${values[2]}"
	if [[ "permanent" != "$status" ]]; then
		continue;
	fi

	# Stitch together the reference.
	reference=''
	if [[ "${#values[@]}" == 5 ]]; then
		reference="${values[3]}"
	else
		unset values[-1] # Remove the comment.
		reference=$(echo "${values[@]:3}" | xargs)
	fi

	reference="$(clean_reference "$reference")"
	const_name="${name^^}"            # To uppercase.
	const_name="${const_name//\-/\_}" # '-' -> '_'.
	const_name="${const_name// /\_}"  # ' ' -> '_'.
	const_value="${name,,}"           # To lowercase.
	value_length="${#const_value}"    # Value length.
	docs="#[doc = \"$name.\\\\n\\\\n$reference.\"]"

	# NOTE: can't assign arrays/list to array values, so we have to use a
	# string and parse that below (which is a little error prone).
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
